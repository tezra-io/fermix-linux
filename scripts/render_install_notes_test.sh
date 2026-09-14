#!/usr/bin/env bash
#
# Exercise render_install_notes.sh, which writes the only page a person reads
# before typing a command as root.
#
# Two states have to be true rather than plausible: paired with a pinned engine
# release, where the commands name files that are on the page; and not paired
# yet, where they must not. A renderer that left a placeholder behind, or that
# printed an engine version nobody published, would put a wrong command in front
# of somebody with sudo.
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
expect_refusal "a version with no pin state" bash "$SCRIPT" 0.1.0
expect_refusal "a pin state that is neither" bash "$SCRIPT" 0.1.0 maybe
expect_refusal "a pinned release that names no engine version" bash "$SCRIPT" 0.11.0 pinned
expect_refusal "an unpinned release that names one anyway" bash "$SCRIPT" 0.1.0 unpinned 0.11.0

echo "render_install_notes_test: the pinned page"
pinned="$(bash "$SCRIPT" 0.11.0 pinned 0.11.0)"
for needed in \
  "sudo apt install ./fermix_0.11.0_" \
  "./fermix-desktop_0.11.0_" \
  "sudo dnf install ./fermix-0.11.0-1." \
  "./fermix-desktop-0.11.0-1." \
  "sudo zypper install" \
  "cosign verify-blob" \
  "fermix-desktop-v0.11.0"; do
  case "$pinned" in
    *"$needed"*) ;;
    *) fail "the pinned page does not carry '$needed'" ;;
  esac
done
case "$pinned" in
  *"{{"*) fail "the pinned page still carries a placeholder" ;;
esac
echo "  ok: both families, both halves, the signing identity, and nothing left unfilled"

echo "render_install_notes_test: the unpinned page"
unpinned="$(bash "$SCRIPT" 0.1.0 unpinned)"
case "$unpinned" in
  *"{{"*) fail "the unpinned page still carries a placeholder" ;;
esac
case "$unpinned" in
  *"not pinned yet"*) ;;
  *) fail "the unpinned page does not say the engine half is missing" ;;
esac
# The page must not name an engine version nobody published: a command naming a
# file that is not on the page is the one thing this page cannot do.
case "$unpinned" in
  *"<engine version>"*) ;;
  *) fail "the unpinned page does not leave the engine version for the reader to fill" ;;
esac
echo "  ok: it says what is missing rather than naming a file that is not there"

echo "render_install_notes_test: what the page never does"
# One proper noun carries an exclamation mark, and it is the distribution's own
# name rather than this page's voice, so it is taken out before the check the way
# the copy catalogue keeps its own hand-listed proper nouns.
for page in "$pinned" "$unpinned"; do
  page="${page//Pop!_OS/Pop_OS}"
  for forbidden in '—' '!'; do
    case "$page" in
      *"$forbidden"*) fail "the page carries $forbidden" ;;
    esac
  done
done
echo "  ok: no em dash, and no exclamation mark outside one distribution's own name"

echo "render_install_notes_test: every refusal fired"
