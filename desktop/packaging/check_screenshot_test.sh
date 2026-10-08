#!/usr/bin/env bash
# Offline tests for check_screenshot.sh, with the host's ImageMagick. The two real captures of the
# release package's window are fixtures: fedora:44's, which is right, is accepted, and the first
# ubuntu:22.04 smoke's, which old netpbm decoded with the wrong stride (skewed, every other row
# black), is refused. So are a screen of the wrong size, a window that draws nothing, pixels outside
# the window, and rows that alternate.
#   desktop/packaging/check_screenshot_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/smoke_fixtures"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# The window as both captures hold it: at the root's corner, 880 by 560.
WINDOW=(0 0 880 560)

fail() {
  echo "check_screenshot_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1" reason="$2" err="$work/stderr"
  shift 2
  if "$@" > /dev/null 2> "$err"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$err" || fail "$what was refused for another reason: $(cat "$err")"
  echo "  refused: $what"
}

check() {
  "$here/check_screenshot.sh" "$1" 1280x800 "${@:2}"
}

command -v convert > /dev/null || fail "ImageMagick's convert is needed to make the fixtures"

# A drawn window: a light surface with dark text and a header, on the black root.
convert -size 1280x800 xc:black \
  \( -size 880x560 xc:'#fafafa' -fill '#ebebeb' -draw 'rectangle 0,0 879,46' \
     -fill '#303030' -pointsize 22 -draw "text 380,30 'Fermix'" \
     -draw "text 40,140 'Status'" -draw "text 40,200 'Answers with'" \) \
  -geometry +0+0 -composite "$work/drawn.png"

echo "check_screenshot_test: a drawn window"
check "$fixtures/drawn-fedora-44.png" "${WINDOW[@]}" > "$work/ok" ||
  fail "fedora:44's capture was refused: $(cat "$work/ok")"
echo "  ok: fedora:44's capture: $(cat "$work/ok")"
check "$work/drawn.png" "${WINDOW[@]}" > "$work/ok" || fail "the drawn window was refused: $(cat "$work/ok")"
echo "  ok: a drawn fixture: $(cat "$work/ok")"

echo "check_screenshot_test: captures it refuses"
expect_refusal "the garbled capture of the first smoke" "rows alternate" \
  check "$fixtures/garbled-xwdtopnm.png" "${WINDOW[@]}"
convert "$work/drawn.png" -resize 1024x640! "$work/small.png"
expect_refusal "a capture of another size" "is 1024x640, not the screen's 1280x800" \
  check "$work/small.png" "${WINDOW[@]}"
convert -size 1280x800 xc:black -fill '#fafafa' -draw 'rectangle 0,0 879,559' "$work/blank.png"
expect_refusal "a window that draws nothing" "draws nothing" check "$work/blank.png" "${WINDOW[@]}"
convert "$work/drawn.png" -fill white -draw 'rectangle 1000,300 1100,400' "$work/outside.png"
expect_refusal "pixels outside the window" "outside the window" check "$work/outside.png" "${WINDOW[@]}"
convert "$work/drawn.png" -fill black -draw 'rectangle 0,0 879,559' -fill '#fafafa' \
  -draw "$(for ((y = 0; y < 560; y += 2)); do printf 'line 0,%d 879,%d ' "$y" "$y"; done)" \
  -fill '#303030' -draw "text 40,140 'Status'" "$work/striped.png"
expect_refusal "a window whose rows alternate" "rows alternate" check "$work/striped.png" "${WINDOW[@]}"
expect_refusal "a window off the screen" "is not on the 1280x800 screen" check "$work/drawn.png" 1300 0 100 100
expect_refusal "no window" "usage" "$here/check_screenshot.sh" "$work/drawn.png" 1280x800

echo "check_screenshot_test: ok"
