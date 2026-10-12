#!/usr/bin/env bash
#
# The screenshot step of desktop/scripts/install_smoke.sh, run in one pinned converter image for
# every distribution, so the image under test never decodes its own capture.
#
#   convert_screenshot.sh <dir> <screen WxH> <x11 | wayland>...
#
# <dir> holds what the smoke copied out, for each display it drew on:
#   x11      screen.xwd, the root window dumped by xwd in the X server's own format, and
#            window-bounds.txt, the mapped window's "x y width height". ImageMagick decodes the dump
#            into screenshot.png, check_screenshot.sh holds it to the window's bounds, and the same
#            check must refuse the control, the garbled capture of the first ubuntu:22.04 smoke.
#            screenshot.png.sha256 is written beside it for the copy out.
#   wayland  wayland-screenshot.png, Weston's output with the window on it, and wayland-empty.png,
#            the same output before the window opened. Weston's kiosk shell gives the window the
#            whole output, so check_screenshot.sh holds the capture to the whole screen, and the
#            same check must refuse the empty output as drawing nothing.
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
GARBLED="$here/smoke_fixtures/garbled-xwdtopnm.png"

fail() {
  echo "convert_screenshot: $*" >&2
  exit 1
}

check_x11() {
  local dir="$1" screen="$2" verdict
  local -a bounds
  [ -f "$dir/screen.xwd" ] && [ -f "$dir/window-bounds.txt" ] || fail "$dir has no screen.xwd and window-bounds.txt"
  read -r -a bounds < "$dir/window-bounds.txt"
  [ "${#bounds[@]}" -eq 4 ] || fail "window-bounds.txt is not 'x y width height': ${bounds[*]}"
  magick "xwd:$dir/screen.xwd" "png:$dir/screenshot.png" || fail "ImageMagick cannot decode screen.xwd"
  bash "$here/check_screenshot.sh" "$dir/screenshot.png" "$screen" "${bounds[@]}" ||
    fail "screenshot.png does not show the window drawn"
  if verdict="$(bash "$here/check_screenshot.sh" "$GARBLED" "$screen" "${bounds[@]}" 2>&1)"; then
    fail "the control was accepted: check_screenshot.sh passed the garbled capture"
  fi
  grep -qF "rows alternate" <<< "$verdict" || fail "the control was refused for another reason: $verdict"
  echo "convert_screenshot: control: the garbled capture is refused: rows alternate"
  (cd "$dir" && sha256sum screenshot.png > screenshot.png.sha256)
}

check_wayland() {
  local dir="$1" screen="$2" verdict
  local -a whole=(0 0 "${screen%x*}" "${screen#*x}")
  [ -f "$dir/wayland-screenshot.png" ] && [ -f "$dir/wayland-empty.png" ] ||
    fail "$dir has no wayland-screenshot.png and wayland-empty.png"
  bash "$here/check_screenshot.sh" "$dir/wayland-screenshot.png" "$screen" "${whole[@]}" ||
    fail "wayland-screenshot.png does not show the window drawn"
  if verdict="$(bash "$here/check_screenshot.sh" "$dir/wayland-empty.png" "$screen" "${whole[@]}" 2>&1)"; then
    fail "the control was accepted: check_screenshot.sh passed the compositor's screen before the window"
  fi
  grep -qF "draws nothing" <<< "$verdict" || fail "the control was refused for another reason: $verdict"
  echo "convert_screenshot: control: the compositor's screen before the window is refused: it draws nothing"
}

main() {
  [ $# -ge 3 ] || fail "usage: convert_screenshot.sh <dir> <screen WxH> <x11 | wayland>..."
  local dir="$1" screen="$2" display version
  shift 2
  for display in "$@"; do
    case "$display" in x11 | wayland) ;; *) fail "the displays are x11 and wayland, not '$display'" ;; esac
  done
  # Read whole, then cut: piped into head, magick takes SIGPIPE on its next line, and pipefail
  # makes that the pipeline's status.
  version="$(magick -version)" || fail "ImageMagick cannot say its version"
  echo "${version%%$'\n'*}" >&2
  for display in "$@"; do
    "check_$display" "$dir" "$screen"
  done
}

main "$@"
