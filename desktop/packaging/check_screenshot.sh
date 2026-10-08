#!/usr/bin/env bash
#
# A screenshot of the smoke's screen shows the window drawn, and was decoded right.
#
#   check_screenshot.sh <png> <screen WxH> <window x> <window y> <window width> <window height>
#
# With ImageMagick (magick, or convert where there is no magick), it checks four things, in order:
#   1. the PNG is the screen's size;
#   2. inside the window's bounds, rows do not alternate: neighbouring rows differ little on
#      average, as they do on a drawn surface (0.007 in fedora:44's capture). A capture decoded with
#      the wrong stride or depth (skewed, with black rows between; 0.66 in the first ubuntu:22.04
#      smoke's) fails here;
#   3. inside the window's bounds, something is drawn: the grey levels spread;
#   4. outside the window's bounds the root is black, as Xvfb -br leaves it.
set -euo pipefail
shopt -s inherit_errexit

# Rows alternate when neighbouring rows differ by more than this on average, in grey from 0 to 1.
MAX_ROW_DIFFERENCE=0.05
# A window whose grey levels spread less than this draws nothing.
MIN_SPREAD=0.02

fail() {
  echo "check_screenshot: $*" >&2
  exit 1
}

magick_of() {
  if command -v magick > /dev/null; then
    echo magick
  elif command -v convert > /dev/null; then
    echo convert
  else
    fail "ImageMagick is not installed"
  fi
}

# One fx expression over <png> cropped to <geometry>, in grey: <tool> <png> <geometry> <format>
measure() {
  "$1" "$2" -colorspace gray -crop "$3" +repage -format "$4" info: ||
    fail "ImageMagick cannot measure $2 at $3"
}

# The mean difference between each row and the next, inside <geometry>.
row_difference() {
  "$1" "$2" -colorspace gray -crop "$3" +repage \( +clone -roll +0+1 \) -compose difference \
    -composite -format '%[fx:mean]' info: || fail "ImageMagick cannot compare the rows of $2"
}

# The regions of the screen outside the window: right of it, then below it, as WxH+X+Y.
outside_regions() {
  local width="$1" height="$2" x="$3" y="$4" w="$5" h="$6"
  [ $((x + w)) -ge "$width" ] || echo "$((width - x - w))x${height}+$((x + w))+0"
  [ $((y + h)) -ge "$height" ] || echo "$((x + w < width ? x + w : width))x$((height - y - h))+0+$((y + h))"
  [ "$x" -eq 0 ] || echo "${x}x${height}+0+0"
  [ "$y" -eq 0 ] || echo "$((x + w < width ? x + w : width))x${y}+0+0"
}

main() {
  [ $# -eq 6 ] || fail "usage: check_screenshot.sh <png> <screen WxH> <x> <y> <width> <height>"
  local png="$1" screen="$2" x="$3" y="$4" w="$5" h="$6" tool size width height window rows spread region brightest
  [ -f "$png" ] || fail "no screenshot at $png"
  tool="$(magick_of)"
  width="${screen%x*}" height="${screen#*x}"
  size="$("$tool" "$png" -format '%wx%h' info:)" || fail "ImageMagick cannot read $png"
  [ "$size" = "$screen" ] || fail "$png is $size, not the screen's $screen"
  [ "$x" -lt "$width" ] && [ "$y" -lt "$height" ] || fail "the window at $x,$y is not on the $screen screen"
  w=$((x + w > width ? width - x : w)) h=$((y + h > height ? height - y : h))
  window="${w}x${h}+${x}+${y}"
  rows="$(row_difference "$tool" "$png" "$window")"
  awk -v rows="$rows" -v most="$MAX_ROW_DIFFERENCE" 'BEGIN { exit !(rows <= most) }' ||
    fail "$png: inside the window, rows alternate: neighbouring rows differ by $rows on average, above $MAX_ROW_DIFFERENCE"
  spread="$(measure "$tool" "$png" "$window" '%[fx:standard_deviation]')"
  awk -v spread="$spread" -v least="$MIN_SPREAD" 'BEGIN { exit !(spread >= least) }' ||
    fail "$png: the window draws nothing: its grey levels spread by $spread"
  while read -r region; do
    brightest="$(measure "$tool" "$png" "$region" '%[fx:maxima]')"
    [ "$brightest" = 0 ] || fail "$png: something is drawn outside the window, at $region"
  done < <(outside_regions "$width" "$height" "$x" "$y" "$w" "$h")
  echo "check_screenshot: $(basename "$png") is $size; the window at $window draws (spread $spread)," \
    "its rows do not alternate (neighbours differ by $rows), and outside it the root is black"
}

main "$@"
