#!/usr/bin/env bash
# Offline tests for convert_screenshot.sh, with a stand-in for ImageMagick 7's magick that hands
# its work to the host's ImageMagick. The stand-in's -version writes its first line, pauses, and
# writes on, as the real one can: a script that reads only that first line through a pipe under
# pipefail gets the writer's SIGPIPE status, 141, and set -e stops it with no sentence of its own.
# The converter has to report the version, decode the capture, accept fedora:44's, refuse the
# garbled control, and write the PNG's sha256.
#   desktop/packaging/convert_screenshot_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
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
mkdir -p "$work/capture"
cp "$here/smoke_fixtures/drawn-fedora-44.png" "$work/capture/screen.xwd"
echo "0 0 880 560" > "$work/capture/window-bounds.txt"

echo "convert_screenshot_test: a capture, with a magick that writes on after its version line"
status=0
PATH="$work/bin:$PATH" "$here/convert_screenshot.sh" "$work/capture" 1280x800 > "$work/out" 2>&1 || status=$?
[ "$status" = 0 ] || fail "the converter stopped with status $status; it said: $(tr '\n' '|' < "$work/out")"
grep -qxF "Version: ImageMagick 7 (the test's stand-in)" "$work/out" ||
  fail "the converter did not report ImageMagick's version: $(cat "$work/out")"
grep -qF "check_screenshot: screenshot.png is 1280x800" "$work/out" ||
  fail "the converter did not check the capture: $(cat "$work/out")"
grep -qxF "convert_screenshot: control: the garbled capture is refused: rows alternate" "$work/out" ||
  fail "the converter did not run its control: $(cat "$work/out")"
(cd "$work/capture" && sha256sum --quiet --check screenshot.png.sha256) ||
  fail "screenshot.png.sha256 does not match screenshot.png"
echo "  ok: the version line, the capture accepted, the control refused, the sha256 written"

echo "convert_screenshot_test: ok"
