#!/usr/bin/env bash
# Offline tests for smoke_in_container.sh's window_mapped, the wait for the window to map, with a
# stand-in for xwininfo. The stand-in reports the window viewable, pauses, and writes on, as the
# real one does for every line after Map State: a check that reads only up to that line through a
# pipe under pipefail gets the writer's SIGPIPE status, 141, takes the window for unmapped, and
# waits out the smoke's 60 seconds. window_mapped has to find the window and write its id.
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

# <map state>: a stand-in xwininfo for one window titled Fermix, 0x400005, in that state.
stand_in() {
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

# window_mapped from the script, in a subshell, with the stand-in first on PATH.
mapped() {
  (
    # shellcheck source=desktop/packaging/smoke_in_container.sh
    source "$here/smoke_in_container.sh"
    trap - ERR
    WINDOW_ID_FILE="$work/window.id"
    PATH="$work/bin:$PATH" window_mapped
  )
}

echo "smoke_in_container_test: a viewable window, from an xwininfo that writes on after Map State"
stand_in IsViewable
rm -f "$work/window.id"
mapped || fail "window_mapped did not find the viewable window"
[ "$(cat "$work/window.id")" = 0x400005 ] || fail "window_mapped wrote the id $(cat "$work/window.id")"
echo "  ok: found, and its id 0x400005 written"

echo "smoke_in_container_test: a window that is not viewable yet"
stand_in IsUnMapped
rm -f "$work/window.id"
if mapped; then
  fail "window_mapped took an unmapped window for mapped"
fi
[ ! -e "$work/window.id" ] || fail "window_mapped wrote an id for an unmapped window"
echo "  ok: not found, and no id written"

echo "smoke_in_container_test: ok"
