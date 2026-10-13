#!/usr/bin/env bash
# Offline tests for smoke_in_container.sh, with stand-ins for the tools it reads the display with.
#
# window_mapped, the wait for the X11 window to map: the stand-in xwininfo reports the window
# viewable, pauses, and writes on, as the real one does for every line after Map State. A check that
# reads only up to that line through a pipe under pipefail gets the writer's SIGPIPE status, 141,
# takes the window for unmapped, and waits out the smoke's 60 seconds.
#
# wayland_capture and wayland_drawn, the Weston captures: a weston-screenshooter that the compositor
# does not authorise says so and still exits 0, writing nothing, so a capture is the file it wrote.
#
# check_wayland_client, which holds the window to the private libwayland-client; tools_from_epel,
# which says where an rpm image's test tools come from; provided_engine, which reads the engine
# version `fermix --version` has to answer with from what the installed package provides; and the
# flags the script accepts.
#   desktop/packaging/smoke_in_container_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "smoke_in_container_test: $*" >&2
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

# <function> <args...> from the script, in a subshell, with the stand-ins first on PATH.
sourced() {
  (
    # shellcheck source=desktop/packaging/smoke_in_container.sh
    source "$here/smoke_in_container.sh"
    trap - ERR
    WINDOW_ID_FILE="$work/window.id" OUT="$work/out" SHOTS="$work/shots" WAYLAND_NOW="$work/now.png"
    PATH="$work/bin:$PATH" "$@"
  )
}

# <map state>: a stand-in xwininfo for one window titled Fermix, 0x400005, in that state.
stand_in_xwininfo() {
  mkdir -p "$work/bin"
  cat > "$work/bin/xwininfo" <<EOF
#!/usr/bin/env bash
if [ "\$1" = -root ]; then
  echo '     0x400003 "fermix-desktop": ("fermix-desktop" "Fermix-desktop")  10x10+0+0  +0+0'
  echo '     0x400005 "Fermix": ("fermix-desktop" "Fermix-desktop")  880x560+0+0  +0+0'
  exit 0
fi
echo "xwininfo: Window id: \$2 \"Fermix\""
echo "  Map State: $1"
sleep 0.5
echo "  Override Redirect State: no" || exit 141
EOF
  chmod 0755 "$work/bin/xwininfo"
}

# <behaviour>: a stand-in weston-screenshooter that writes the file $work/screen into the working
# directory under the name Weston 14 gives it, or is refused as unauthorised and exits 0, as the
# real one does when Weston runs without --debug, or fails as one with no compositor does.
stand_in_screenshooter() {
  mkdir -p "$work/bin"
  case "$1" in
    writes) printf '#!/bin/sh\ncp "%s" "wayland-screenshot-2026-10-12_02-22-20.png"\n' "$work/screen" ;;
    refused) printf '#!/bin/sh\necho "Output capture error: unauthorized" >&2\nexit 0\n' ;;
    failing) printf '#!/bin/sh\necho "failed to create display" >&2\nexit 1\n' ;;
  esac > "$work/bin/weston-screenshooter"
  chmod 0755 "$work/bin/weston-screenshooter"
}

mkdir -p "$work/out"

echo "smoke_in_container_test: a viewable window, from an xwininfo that writes on after Map State"
stand_in_xwininfo IsViewable
rm -f "$work/window.id"
sourced window_mapped || fail "window_mapped did not find the viewable window"
[ "$(cat "$work/window.id")" = 0x400005 ] || fail "window_mapped wrote the id $(cat "$work/window.id")"
echo "  ok: found, and its id 0x400005 written"

echo "smoke_in_container_test: a window that is not viewable yet"
stand_in_xwininfo IsUnMapped
rm -f "$work/window.id"
if sourced window_mapped; then
  fail "window_mapped took an unmapped window for mapped"
fi
[ ! -e "$work/window.id" ] || fail "window_mapped wrote an id for an unmapped window"
echo "  ok: not found, and no id written"

echo "smoke_in_container_test: a Weston capture"
printf 'the window drawn\n' > "$work/screen"
stand_in_screenshooter writes
sourced wayland_capture "$work/captured.png" || fail "wayland_capture did not take the capture"
[ "$(cat "$work/captured.png")" = "the window drawn" ] || fail "wayland_capture kept another file"
echo "  ok: the screenshooter's file, moved to where it was asked for"
for behaviour in refused failing; do
  stand_in_screenshooter "$behaviour"
  rm -f "$work/captured.png"
  if sourced wayland_capture "$work/captured.png"; then
    fail "wayland_capture took a capture when the screenshooter was $behaviour"
  fi
  [ ! -e "$work/captured.png" ] || fail "wayland_capture left a file when the screenshooter was $behaviour"
  echo "  ok: no capture when the screenshooter is $behaviour"
done

echo "smoke_in_container_test: the wait for the window to draw on Weston"
printf 'the empty output\n' > "$work/out/wayland-empty.png"
cp "$work/out/wayland-empty.png" "$work/screen"
stand_in_screenshooter writes
if sourced wayland_drawn; then
  fail "wayland_drawn took the empty output for a window"
fi
printf 'the window drawn\n' > "$work/screen"
sourced wayland_drawn || fail "wayland_drawn did not see the window"
stand_in_screenshooter refused
if sourced wayland_drawn; then
  fail "wayland_drawn took a refused capture for a window"
fi
echo "  ok: the empty output and a refused capture are not a window; a changed output is"

echo "smoke_in_container_test: the window's libwayland-client"
private=/usr/lib/fermix-desktop/lib/libwayland-client.so.0.26.0
host=/usr/lib/x86_64-linux-gnu/libwayland-client.so.0.22.0
mapping='7f0000000000-7f0000001000 r--p 00000000 00:2a 123'
printf '%s %s\n' "$mapping" "$private" "$mapping" "$private" > "$work/maps-private"
printf '%s %s\n' "$mapping" "$private" "$mapping" "$host" > "$work/maps-both"
printf '%s %s\n' "$mapping" /usr/lib/fermix-desktop/lib/libgtk-4.so.1 > "$work/maps-none"
[ "$(sourced check_wayland_client "$work/maps-private")" = "$private" ] ||
  fail "check_wayland_client did not name the private libwayland-client once"
echo "  ok: the private one, named once"
expect_refusal "a window that also loads the host's libwayland-client" \
  "the window loads $host, not libwayland-client from /usr/lib/fermix-desktop/lib" \
  sourced check_wayland_client "$work/maps-both"
expect_refusal "a window with no libwayland-client" "the window has no libwayland-client loaded" \
  sourced check_wayland_client "$work/maps-none"

echo "smoke_in_container_test: where an rpm image's test tools come from"
printf 'NAME="AlmaLinux"\nID="almalinux"\nVERSION_ID="10.2"\n' > "$work/os-almalinux"
printf 'NAME="Fedora Linux"\nID=fedora\nVERSION_ID=44\n' > "$work/os-fedora"
printf 'NAME="Rocky Linux"\nID="rocky"\n' > "$work/os-rocky"
sourced tools_from_epel "$work/os-almalinux" || fail "AlmaLinux's test tools are not taken from EPEL"
if sourced tools_from_epel "$work/os-fedora"; then
  fail "Fedora's test tools are taken from EPEL"
fi
echo "  ok: AlmaLinux's from EPEL, Fedora's from its own archive"
expect_refusal "a distribution the smoke has no rule for" \
  "the smoke knows where fedora's and almalinux's test tools come from, not rocky's" \
  sourced tools_from_epel "$work/os-rocky"

# A stand-in dpkg-query and rpm that answer what fermix-desktop provides with $PROVIDES_FILE: the
# deb's Provides field, or the rpm's provides one a line.
mkdir -p "$work/bin"
export PROVIDES_FILE="$work/provides"
cat > "$work/bin/dpkg-query" << 'STUB'
#!/usr/bin/env bash
[ "$*" = '-W -f ${Provides} fermix-desktop' ] || { echo "stand-in dpkg-query was asked: $*" >&2; exit 2; }
cat "$PROVIDES_FILE"
STUB
cat > "$work/bin/rpm" << 'STUB'
#!/usr/bin/env bash
[ "$*" = '-q --provides fermix-desktop' ] || { echo "stand-in rpm was asked: $*" >&2; exit 2; }
cat "$PROVIDES_FILE"
STUB
chmod 0755 "$work/bin/dpkg-query" "$work/bin/rpm"

# <family> <provides> <engine>: the engine version read from <provides> is <engine>.
expect_engine() {
  printf '%s\n' "$2" > "$PROVIDES_FILE"
  [ "$(sourced provided_engine "$1")" = "$3" ] || fail "the $1's provides '$2' did not give the engine $3"
}

echo "smoke_in_container_test: the engine version the installed package provides"
expect_engine deb 'fermix (= 0.14.0)' 0.14.0
expect_engine deb 'libfoo, fermix (= 0.14.0)' 0.14.0
expect_engine rpm "$(printf 'fermix = 0.14.0\nfermix-desktop = 0.1.0-1\nfermix-desktop(x86-64) = 0.1.0-1')" 0.14.0
echo "  ok: 0.14.0, from the deb's Provides and from the rpm's provides"
for provides in 'fermix' 'fermix-desktop (= 0.1.0)' '' 'fermix (= 0.14.0), fermix (= 0.15.0)'; do
  printf '%s\n' "$provides" > "$PROVIDES_FILE"
  expect_refusal "a deb that provides '$provides'" "fermix-desktop does not provide fermix at one version" \
    sourced provided_engine deb
done
for provides in 'fermix-desktop = 0.1.0-1' 'fermix' "$(printf 'fermix = 0.14.0\nfermix = 0.15.0')"; do
  printf '%s\n' "$provides" > "$PROVIDES_FILE"
  expect_refusal "an rpm that provides '${provides//$'\n'/, }'" "fermix-desktop does not provide fermix at one version" \
    sourced provided_engine rpm
done

echo "smoke_in_container_test: the flags it accepts"
run_script() {
  bash "$here/smoke_in_container.sh" --family rpm --package /smoke/p.rpm --sha256 0 "$@"
}
expect_refusal "a display it does not know" "the displays are x11 and wayland, not 'mir'" \
  run_script --displays mir
expect_refusal "one it knows beside one it does not" "not 'mir'" run_script --displays "x11 mir"
expect_refusal "no display" "every flag needs a value" run_script --displays ""
expect_refusal "no --displays" "every flag needs a value" run_script
expect_refusal "an engine version named by the caller: the package says it" "usage" \
  run_script --displays x11 --engine-version 0.14.0

echo "smoke_in_container_test: ok"
