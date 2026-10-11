#!/usr/bin/env bash
#
# The half of desktop/scripts/install_smoke.sh that runs in a clean ubuntu:22.04 or fedora:44
# container. It checks the package copied in against its sha256 and installs it with apt or dnf,
# which brings in exactly what the package declares. Then, before anything else is installed:
# `fermix --version` answers with the engine's version, and ldd resolves every ELF object under
# /usr/lib/fermix-desktop, each private library from the prefix. A control copies the window out of
# the prefix and expects that check to refuse it. Only then are Xvfb, a session bus and the
# screenshot tools installed, and the window drawn in the state it shows when Fermix is not
# running, and the screen dumped by xwd in the X server's own format with the window's bounds.
# install_smoke.sh decodes and checks that dump in one pinned converter image, the same for every
# distribution. What this writes lands in /smoke/out with a SHA256SUMS.
#
# Usage: smoke_in_container.sh --family <deb|rpm> --package <file> --sha256 <hex> --engine-version <v>
set -eEuo pipefail
shopt -s inherit_errexit
# A command that stops the smoke with no sentence of its own still names itself.
trap 'echo "smoke_in_container: line $LINENO: \"$BASH_COMMAND\" failed with $?" >&2' ERR

PREFIX=/usr/lib/fermix-desktop
WINDOW="$PREFIX/bin/fermix-desktop"
OUT=/smoke/out
DISPLAY_NUMBER=99
SCREEN=1280x800
# Seconds to wait for Xvfb's socket and for the window to map; then the smoke fails.
DISPLAY_WAIT_SECONDS=30
WINDOW_WAIT_SECONDS=60
# Seconds the mapped window is given to ask for Fermix's state and draw the answer.
SETTLE_SECONDS=8
WINDOW_PID_FILE=/tmp/window.pid
WINDOW_ID_FILE=/tmp/window.id
XVFB_PID_FILE=/tmp/xvfb.pid

fail() {
  echo "smoke_in_container: $*" >&2
  exit 1
}

log() {
  echo "smoke_in_container: $*" >&2
}

install_package() {
  local family="$1" package="$2" digest="$3"
  echo "$digest  $package" | sha256sum --quiet --check - || fail "$package is not the package copied in"
  case "$family" in
    deb)
      export DEBIAN_FRONTEND=noninteractive
      apt-get -o Acquire::Retries=3 update > "$OUT/install.log" 2>&1 || fail "apt-get update failed"
      apt-get -o Acquire::Retries=3 install -y "$package" >> "$OUT/install.log" 2>&1 ||
        fail "apt-get cannot install $package: $(tail -n 20 "$OUT/install.log")"
      ;;
    rpm)
      dnf install -y "$package" > "$OUT/install.log" 2>&1 ||
        fail "dnf cannot install $package: $(tail -n 20 "$OUT/install.log")"
      ;;
  esac
  log "installed $(basename -- "$package") and what it declares: see install.log"
}

check_engine() {
  local engine="$1" answer
  answer="$(fermix --version 2>&1)" || fail "fermix --version failed: $answer"
  grep -qF -- "$engine" <<< "$answer" || fail "fermix --version answers '$answer', not $engine"
  printf '%s\n' "$answer" > "$OUT/fermix-version.txt"
  log "fermix --version: $answer"
}

is_elf() {
  [ "$(od -An -tx1 -N4 -- "$1" | tr -d ' \n')" = 7f454c46 ]
}

# ldd resolves <object> whole, and every library the prefix carries from the prefix.
check_object() {
  local object="$1" report name path
  report="$(ldd -- "$object" 2>&1)" || fail "ldd cannot read $object: $report"
  if grep -q 'not found' <<< "$report"; then
    fail "ldd cannot resolve $object: $(grep 'not found' <<< "$report" | tr -s ' \t\n' ' ')"
  fi
  while read -r name path; do
    [ -e "$PREFIX/lib/$name" ] || continue
    [ "$(realpath -- "$path")" = "$(realpath -- "$PREFIX/lib/$name")" ] ||
      fail "$object loads $name from $path, not from the private prefix"
  done < <(sed -n 's/^[[:space:]]*\([^ ]*\) => \([^ ]*\) (0x.*/\1 \2/p' <<< "$report")
}

# Every file under the prefix, by glob: a base image need not have find.
check_libraries() {
  local object checked=0 name
  [ "$(readlink -f /usr/bin/fermix-desktop)" = "$WINDOW" ] ||
    fail "/usr/bin/fermix-desktop is not a link to $WINDOW"
  shopt -s globstar nullglob
  for object in "$PREFIX"/**; do
    if [ -L "$object" ] || [ ! -f "$object" ] || ! is_elf "$object"; then
      continue
    fi
    check_object "$object"
    checked=$((checked + 1))
  done
  shopt -u globstar nullglob
  ldd "$WINDOW" > "$OUT/ldd-fermix-desktop.txt"
  for name in libgtk-4.so.1 libadwaita-1.so.0 libglib-2.0.so.0; do
    grep -q "^[[:space:]]*$name => $PREFIX/" "$OUT/ldd-fermix-desktop.txt" ||
      fail "the window does not load $name from $PREFIX"
  done
  log "ldd resolves all $checked ELF objects under $PREFIX, GTK, libadwaita and GLib from the prefix"
}

# The control: the same window outside the prefix finds none of its private libraries.
check_control() {
  local copy=/tmp/control/bin/fermix-desktop reason
  mkdir -p "$(dirname "$copy")"
  cp -- "$WINDOW" "$copy"
  if reason="$( (check_object "$copy") 2>&1)"; then
    fail "the control was accepted: a window outside the prefix resolved"
  fi
  grep -qF "ldd cannot resolve $copy" <<< "$reason" || fail "the control was refused for another reason: $reason"
  rm -rf /tmp/control
  log "control: the window copied out of the prefix is refused"
}

install_display_tools() {
  local family="$1"
  case "$family" in
    deb)
      apt-get install -y --no-install-recommends xvfb x11-apps x11-utils dbus \
        >> "$OUT/display-tools.log" 2>&1 || fail "apt-get cannot install the display tools"
      ;;
    rpm)
      dnf install -y /usr/bin/Xvfb /usr/bin/xwd /usr/bin/xwininfo /usr/bin/dbus-run-session \
        >> "$OUT/display-tools.log" 2>&1 || fail "dnf cannot install the display tools"
      ;;
  esac
}

# Waits up to <seconds> for <command...> to succeed.
wait_for() {
  local seconds="$1" second
  shift
  for ((second = 0; second < seconds; second++)); do
    "$@" && return 0
    sleep 1
  done
  return 1
}

# A window titled Fermix that the X server shows, its id written to $WINDOW_ID_FILE; GTK may also
# name windows it never maps. Each report is read whole before it is searched: piped into grep -q,
# xwininfo takes SIGPIPE on the lines after Map State, and pipefail makes that the pipeline's status.
window_mapped() {
  local id report
  for id in $(xwininfo -root -tree 2> /dev/null | sed -n 's/^[[:space:]]*\(0x[0-9a-f]*\) "Fermix":.*/\1/p'); do
    report="$(xwininfo -id "$id" 2> /dev/null)" || continue
    if grep -q 'Map State: IsViewable' <<< "$report"; then
      echo "$id" > "$WINDOW_ID_FILE"
      return 0
    fi
  done
  return 1
}

window_alive() {
  [ -s "$WINDOW_PID_FILE" ] && kill -0 "$(cat "$WINDOW_PID_FILE")" 2> /dev/null
}

start_display() {
  # -br: a black root, so check_screenshot.sh can tell the window's pixels from the rest.
  Xvfb ":$DISPLAY_NUMBER" -screen 0 "${SCREEN}x24" -br -nolisten tcp > "$OUT/xvfb.log" 2>&1 &
  echo "$!" > "$XVFB_PID_FILE"
  wait_for "$DISPLAY_WAIT_SECONDS" test -S "/tmp/.X11-unix/X$DISPLAY_NUMBER" ||
    fail "Xvfb did not start: $(cat "$OUT/xvfb.log")"
}

draw_window() {
  # GSK_DEBUG=renderer has GTK say in window.log which renderer drew.
  export DISPLAY=":$DISPLAY_NUMBER" GDK_BACKEND=x11 GSK_DEBUG=renderer LANG=C.UTF-8 XDG_RUNTIME_DIR=/tmp/runtime
  mkdir -m 0700 "$XDG_RUNTIME_DIR"
  # shellcheck disable=SC2016 # $$ is the shell's that becomes the window
  dbus-run-session -- sh -c 'echo $$ > "$1" && exec fermix-desktop' sh "$WINDOW_PID_FILE" \
    > "$OUT/window.log" 2>&1 &
  wait_for "$WINDOW_WAIT_SECONDS" window_mapped ||
    fail "the window did not map in ${WINDOW_WAIT_SECONDS}s: $(tail -n 20 "$OUT/window.log")"
  sleep "$SETTLE_SECONDS"
  window_alive || fail "the window exited after it mapped: $(tail -n 20 "$OUT/window.log")"
  log "the window mapped and is running"
}

# The mapped window's bounds on the root: xwininfo's report into window-geometry.txt, and
# "x y width height" into window-bounds.txt.
window_bounds() {
  local info bounds
  info="$(xwininfo -id "$(cat "$WINDOW_ID_FILE")")" || fail "xwininfo cannot read the window"
  printf '%s\n' "$info" > "$OUT/window-geometry.txt"
  bounds="$(sed -n -e 's/^ *Absolute upper-left X: *\([0-9]*\)$/\1/p' -e 's/^ *Absolute upper-left Y: *\([0-9]*\)$/\1/p' \
    -e 's/^ *Width: *\([0-9]*\)$/\1/p' -e 's/^ *Height: *\([0-9]*\)$/\1/p' <<< "$info" | tr '\n' ' ')"
  [[ "$bounds" =~ ^[0-9]+\ [0-9]+\ [0-9]+\ [0-9]+\ $ ]] || fail "xwininfo gave the window no bounds: $bounds"
  printf '%s\n' "${bounds% }" > "$OUT/window-bounds.txt"
}

# The root window as xwd dumps it, in the X server's own format, into screen.xwd. It is decoded
# outside this image, by desktop/packaging/convert_screenshot.sh in the smoke's converter image.
capture_screen() {
  xwd -root -silent > "$OUT/screen.xwd" 2> "$OUT/xwd.log" || fail "xwd cannot capture the screen: $(cat "$OUT/xwd.log")"
  window_bounds
  log "screen.xwd: the $SCREEN screen as the X server holds it; the window at $(cat "$OUT/window-bounds.txt")"
}

exited() {
  ! kill -0 "$1" 2> /dev/null
}

# Stops the process <pid file> names, and kills it if it has not exited in 10 seconds.
stop() {
  local pid
  pid="$(cat "$1")"
  kill "$pid" || fail "process $pid, from $1, had already exited"
  wait_for 10 exited "$pid" || kill -KILL "$pid" || exited "$pid" || fail "process $pid, from $1, will not stop"
}

write_sums() {
  local sums
  sums="$(cd "$OUT" && sha256sum -- *)"
  printf '%s\n' "$sums" > "$OUT/SHA256SUMS"
}

main() {
  local family="" package="" digest="" engine=""
  while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || fail "usage: smoke_in_container.sh --family <deb|rpm> --package <file> --sha256 <hex> --engine-version <v>"
    case "$1" in
      --family) family="$2" ;; --package) package="$2" ;; --sha256) digest="$2" ;;
      --engine-version) engine="$2" ;;
      *) fail "usage: smoke_in_container.sh --family <deb|rpm> --package <file> --sha256 <hex> --engine-version <v>" ;;
    esac
    shift 2
  done
  case "$family" in deb | rpm) ;; *) fail "the family is deb or rpm, not '$family'" ;; esac
  [ -n "$package" ] && [ -n "$digest" ] && [ -n "$engine" ] || fail "every flag needs a value"
  mkdir -p "$OUT"
  install_package "$family" "$package" "$digest"
  check_engine "$engine"
  check_libraries
  check_control
  install_display_tools "$family"
  start_display
  draw_window
  capture_screen
  stop "$WINDOW_PID_FILE"
  stop "$XVFB_PID_FILE"
  write_sums
}

# Run, not sourced: smoke_in_container_test.sh sources this file for its functions.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
