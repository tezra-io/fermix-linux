#!/usr/bin/env bash
# Offline tests for convert_screenshot.sh, with a stand-in for ImageMagick 7's magick that hands
# its work to the host's ImageMagick. The stand-in's -version writes its first line, pauses, and
# writes on, as the real one can: a script that reads only that first line through a pipe under
# pipefail gets the writer's SIGPIPE status, 141, and set -e stops it with no sentence of its own.
# For X11 the converter has to report the version, decode the capture, accept fedora:44's, refuse
# the garbled control, and write the PNG's sha256. For Wayland it has to accept fedora:44's capture
# from a headless Weston and refuse that compositor's capture from before the window opened.
#   desktop/packaging/convert_screenshot_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/smoke_fixtures"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "convert_screenshot_test: $*" >&2
  exit 1
}

command -v convert > /dev/null || fail "ImageMagick's convert is needed behind the stand-in"

# The stand-in: the "xwd" here is a PNG, so decoding it is a copy through convert.
mkdir -p "$work/bin"
cat > "$work/bin/magick" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  -version)
    echo "Version: ImageMagick 7 (the test's stand-in)"
    sleep 0.5
    echo "Copyright: what the real one prints after its first line" || exit 141
    ;;
  xwd:*) exec convert "png:${1#xwd:}" "$2" ;;
  *) exec convert "$@" ;;
esac
EOF
chmod 0755 "$work/bin/magick"
convert -size 1280x800 xc:black "$work/black.png"

# <dir>: what an X11 pass copies out, fedora:44's capture standing in for the xwd dump.
x11_capture() {
  mkdir -p "$1"
  cp "$fixtures/drawn-fedora-44.png" "$1/screen.xwd"
  echo "0 0 880 560" > "$1/window-bounds.txt"
}

# <dir> <screenshot> <empty>: what a Wayland pass copies out, the capture with the window and the
# compositor's capture from before it opened.
wayland_capture() {
  mkdir -p "$1"
  cp "$2" "$1/wayland-screenshot.png"
  cp "$3" "$1/wayland-empty.png"
}

# <dir> <display>...: the converter on <dir>, its output in $work/out; returns its status.
run_converter() {
  local status=0
  PATH="$work/bin:$PATH" "$here/convert_screenshot.sh" "$1" 1280x800 "${@:2}" > "$work/out" 2>&1 || status=$?
  return "$status"
}

expect_line() {
  grep -qxF -- "$1" "$work/out" || fail "the converter did not say '$1'; it said: $(tr '\n' '|' < "$work/out")"
}

# <what> <reason> <dir> <display>...
expect_refusal() {
  local what="$1" reason="$2"
  shift 2
  if run_converter "$@"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$work/out" || fail "$what was refused for another reason: $(cat "$work/out")"
  echo "  refused: $what"
}

echo "convert_screenshot_test: an X11 capture, with a magick that writes on after its version line"
x11_capture "$work/x11"
run_converter "$work/x11" x11 || fail "the converter stopped; it said: $(tr '\n' '|' < "$work/out")"
expect_line "Version: ImageMagick 7 (the test's stand-in)"
grep -qF "check_screenshot: screenshot.png is 1280x800" "$work/out" ||
  fail "the converter did not check the capture: $(cat "$work/out")"
expect_line "convert_screenshot: control: the garbled capture is refused: rows alternate"
(cd "$work/x11" && sha256sum --quiet --check screenshot.png.sha256) ||
  fail "screenshot.png.sha256 does not match screenshot.png"
echo "  ok: the version line, the capture accepted, the control refused, the sha256 written"

echo "convert_screenshot_test: a Wayland capture from a headless Weston"
wayland_capture "$work/wayland" "$fixtures/drawn-wayland-fedora-44.png" "$work/black.png"
run_converter "$work/wayland" wayland || fail "the converter stopped; it said: $(tr '\n' '|' < "$work/out")"
grep -qF "check_screenshot: wayland-screenshot.png is 1280x800; the window at 1280x800+0+0 draws" "$work/out" ||
  fail "the converter did not check the Wayland capture over the whole output: $(cat "$work/out")"
expect_line "convert_screenshot: control: the compositor's screen before the window is refused: it draws nothing"
[ ! -e "$work/wayland/screenshot.png" ] || fail "a Wayland pass wrote an X11 screenshot.png"
echo "  ok: the capture accepted over the whole output, the empty screen refused"

echo "convert_screenshot_test: both displays in one directory"
x11_capture "$work/both"
wayland_capture "$work/both" "$fixtures/drawn-wayland-fedora-44.png" "$work/black.png"
run_converter "$work/both" x11 wayland || fail "the converter stopped; it said: $(tr '\n' '|' < "$work/out")"
expect_line "convert_screenshot: control: the garbled capture is refused: rows alternate"
expect_line "convert_screenshot: control: the compositor's screen before the window is refused: it draws nothing"
echo "  ok: each capture checked, with its own control"

echo "convert_screenshot_test: what it refuses"
wayland_capture "$work/undrawn" "$work/black.png" "$work/black.png"
expect_refusal "a Wayland window that never drew" "wayland-screenshot.png does not show the window drawn" \
  "$work/undrawn" wayland
wayland_capture "$work/lit" "$fixtures/drawn-wayland-fedora-44.png" "$fixtures/drawn-wayland-fedora-44.png"
expect_refusal "a compositor whose empty screen draws" "the control was accepted" "$work/lit" wayland
expect_refusal "a Wayland pass with no captures" "has no wayland-screenshot.png and wayland-empty.png" \
  "$work/x11" wayland
expect_refusal "an X11 pass with no dump" "has no screen.xwd and window-bounds.txt" "$work/wayland" x11
expect_refusal "no display" "usage" "$work/x11"
expect_refusal "a display it does not know" "the displays are x11 and wayland, not 'mir'" "$work/x11" mir

echo "convert_screenshot_test: ok"
