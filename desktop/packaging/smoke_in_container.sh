#!/usr/bin/env bash
#
# The half of desktop/scripts/install_smoke.sh that runs in a clean container of one of the images
# it runs on. It checks the package copied in against its sha256 and installs it with apt or dnf,
# which brings in exactly what the package declares from the distribution's own archive. Then,
# before anything else is installed: `fermix --version` answers with the version of the engine the
# package carries, which is the version the installed package provides fermix at, as dpkg-query or
# rpm reads it here; and ldd resolves every ELF object under /usr/lib/fermix-desktop, each private
# library from the prefix. A control copies the window out of the prefix and expects that check to
# refuse it. Only then are the display tools installed, and the window drawn on each display it is
# given, in the state it shows when Fermix is not running:
#   x11      under Xvfb, the screen dumped by xwd in the X server's own format with the window's
#            bounds;
#   wayland  on a headless Weston, whose kiosk shell gives the window the whole output, captured by
#            weston-screenshooter before the window opens and after it draws, with the window's
#            libwayland-client read from its mappings: it must be the private prefix's.
# install_smoke.sh checks every capture in one pinned converter image, the same for every
# distribution. What this writes lands in /smoke/out with a SHA256SUMS.
#
# The upgrade, N to N+1, runs under systemd, which --boot installs in the container and runs as its
# first process. install_smoke.sh then starts --upgrade-from beside it. The previous package is
# installed the same way, with `fermix --version` answering with its engine. An account `smoke`
# lingers, and runs `fermix service install` as a person does, which enables fermix.service in its
# user manager and waits for the engine to answer. Then the new package is installed over the
# running engine, with apt or dnf. The installed package must be the new one, and `fermix --version`
# must answer with the new engine, while `fermix status` still names the old engine, with the same
# pid. After `systemctl --user daemon-reload` and a restart of fermix.service, `fermix status` must
# name the new engine, with another pid. The libraries check and its control must still hold. The
# account's journal is kept in account.journal, whether the upgrade passes or fails.
#
# Usage: smoke_in_container.sh --family <deb|rpm> --package <file> --sha256 <hex>
#          --displays "<x11 | wayland | x11 wayland>"
#        smoke_in_container.sh --boot <deb|rpm>
#        smoke_in_container.sh --family <deb|rpm> --package <file> --sha256 <hex>
#          --upgrade-from <file> --previous-sha256 <hex>
set -eEuo pipefail
shopt -s inherit_errexit
# A command that stops the smoke with no sentence of its own still names itself.
trap 'echo "smoke_in_container: line $LINENO: \"$BASH_COMMAND\" failed with $?" >&2' ERR

PREFIX=/usr/lib/fermix-desktop
WINDOW="$PREFIX/bin/fermix-desktop"
OUT=/smoke/out
DISPLAY_NUMBER=99
SCREEN=1280x800
# Seconds to wait for Xvfb's or Weston's socket and for the window to show; then the smoke fails.
DISPLAY_WAIT_SECONDS=30
WINDOW_WAIT_SECONDS=60
# Seconds the mapped window is given to ask for Fermix's state and draw the answer.
SETTLE_SECONDS=8
WINDOW_PID_FILE=/tmp/window.pid
WINDOW_ID_FILE=/tmp/window.id
XVFB_PID_FILE=/tmp/xvfb.pid
WAYLAND_SOCKET=wayland-smoke
WAYLAND_WINDOW_PID_FILE=/tmp/wayland-window.pid
WESTON_PID_FILE=/tmp/weston.pid
# Where weston-screenshooter writes, and the capture the wait for the window compares.
SHOTS=/tmp/shots
WAYLAND_NOW=/tmp/wayland-now.png
# The account the upgrade runs the engine as, and its home.
ACCOUNT=smoke
ACCOUNT_HOME=/home/smoke
# Seconds to wait for the account's user manager, and for the engine to answer `fermix status` with
# the version it should; a new engine's first start extracts its payload. Then the upgrade fails.
USER_MANAGER_WAIT_SECONDS=60
ENGINE_WAIT_SECONDS=180
# The last answer of `fermix status`, which a failed wait quotes.
STATUS_ANSWER=/tmp/fermix-status.txt

fail() {
  echo "smoke_in_container: $*" >&2
  exit 1
}

log() {
  echo "smoke_in_container: $*" >&2
}

# <family> <package> <sha256> <log name>: installs the package, or upgrades to it over one installed.
install_package() {
  local family="$1" package="$2" digest="$3" log="$OUT/$4"
  echo "$digest  $package" | sha256sum --quiet --check - || fail "$package is not the package copied in"
  case "$family" in
    deb)
      export DEBIAN_FRONTEND=noninteractive
      apt-get -o Acquire::Retries=3 update > "$log" 2>&1 || fail "apt-get update failed"
      apt-get -o Acquire::Retries=3 install -y "$package" >> "$log" 2>&1 ||
        fail "apt-get cannot install $package: $(tail -n 20 "$log")"
      ;;
    rpm)
      dnf install -y "$package" > "$log" 2>&1 || fail "dnf cannot install $package: $(tail -n 20 "$log")"
      ;;
  esac
  log "installed $(basename -- "$package") and what it declares: see $4"
}

# The version of fermix-desktop that is installed, or that <package> carries.
installed_version() {
  # shellcheck disable=SC2016 # dpkg-query's field, not the shell's
  case "$1" in
    deb) dpkg-query -W -f '${Version}' fermix-desktop ;;
    rpm) rpm -q --qf '%{VERSION}' fermix-desktop ;;
  esac
}

package_version() {
  case "$1" in
    deb) dpkg-deb -f "$2" Version ;;
    rpm) rpm -qp --qf '%{VERSION}' "$2" ;;
  esac
}

# The version the installed fermix-desktop provides fermix at: the one fermix (= <v>) of the deb's
# Provides, or the one fermix = <v> among the rpm's provides.
provided_engine() {
  local family="$1" provides versions
  case "$family" in
    deb)
      # shellcheck disable=SC2016 # dpkg-query's field, not the shell's
      provides="$(dpkg-query -W -f '${Provides}' fermix-desktop)" || fail "dpkg-query cannot read fermix-desktop"
      versions="$(tr ',' '\n' <<< "$provides" | sed -n 's/^ *fermix (= \([^ )]*\)) *$/\1/p')"
      ;;
    rpm)
      provides="$(rpm -q --provides fermix-desktop)" || fail "rpm cannot read fermix-desktop"
      versions="$(sed -n 's/^fermix = \([^ ]*\)$/\1/p' <<< "$provides")"
      ;;
    *) fail "the family is deb or rpm, not '$family'" ;;
  esac
  [[ "$versions" =~ ^[^[:space:]]+$ ]] ||
    fail "fermix-desktop does not provide fermix at one version: $(tr '\n' ' ' <<< "$provides")"
  echo "$versions"
}

# <engine> <file>: `fermix --version` answers with <engine>, kept in <file> under the output.
check_engine() {
  local engine="$1" answer
  answer="$(fermix --version 2>&1)" || fail "fermix --version failed: $answer"
  grep -qF -- "$engine" <<< "$answer" || fail "fermix --version answers '$answer', not $engine"
  printf '%s\n' "$answer" > "$OUT/$2"
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

# Whether an rpm image takes its display tools from EPEL, by the ID in <os-release>: AlmaLinux 9
# has no xwd and AlmaLinux 10 no Weston in its own archive. Fedora has both.
tools_from_epel() {
  local id
  id="$(sed -n 's/^ID="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' "$1")"
  case "$id" in
    almalinux) return 0 ;;
    fedora) return 1 ;;
    *) fail "the smoke knows where fedora's and almalinux's test tools come from, not $id's" ;;
  esac
}

# The tools <family> installs to draw on <display> and capture it.
display_tools() {
  case "$1 $2" in
    "deb x11") echo xvfb x11-apps x11-utils ;;
    "deb wayland") echo weston ;;
    "rpm x11") echo /usr/bin/Xvfb /usr/bin/xwd /usr/bin/xwininfo ;;
    "rpm wayland") echo /usr/bin/weston /usr/bin/weston-screenshooter ;;
    *) fail "there are no display tools for $2 in the $1 family" ;;
  esac
}

# The display tools, a session bus, and nothing they only recommend. They come after the package,
# so EPEL, where an image needs it, is enabled only once the package's own relations are resolved.
install_display_tools() {
  local family="$1" displays="$2" display names="" log="$OUT/display-tools.log"
  local -a tools
  for display in $displays; do
    names="$names $(display_tools "$family" "$display")"
  done
  case "$family" in
    deb)
      read -r -a tools <<< "dbus $names"
      apt-get install -y --no-install-recommends "${tools[@]}" >> "$log" 2>&1 ||
        fail "apt-get cannot install the display tools: $(tail -n 20 "$log")"
      ;;
    rpm)
      read -r -a tools <<< "/usr/bin/dbus-run-session $names"
      if tools_from_epel /etc/os-release; then
        dnf install -y epel-release >> "$log" 2>&1 || fail "dnf cannot enable EPEL: $(tail -n 20 "$log")"
      fi
      dnf install -y --setopt=install_weak_deps=False "${tools[@]}" >> "$log" 2>&1 ||
        fail "dnf cannot install the display tools: $(tail -n 20 "$log")"
      ;;
  esac
  log "installed the display tools: ${tools[*]}"
}

# Tries <command...> once a second, <seconds> times at most, until it succeeds.
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

# The process <pid file> names is running.
alive() {
  [ -s "$1" ] && kill -0 "$(cat "$1")" 2> /dev/null
}

# What every display shares: a runtime directory, and GSK_DEBUG=renderer, which has GTK say in the
# window's log which renderer drew.
prepare_session() {
  export LANG=C.UTF-8 GSK_DEBUG=renderer XDG_RUNTIME_DIR=/tmp/runtime
  mkdir -m 0700 "$XDG_RUNTIME_DIR"
}

start_display() {
  # -br: a black root, so check_screenshot.sh can tell the window's pixels from the rest.
  Xvfb ":$DISPLAY_NUMBER" -screen 0 "${SCREEN}x24" -br -nolisten tcp > "$OUT/xvfb.log" 2>&1 &
  echo "$!" > "$XVFB_PID_FILE"
  wait_for "$DISPLAY_WAIT_SECONDS" test -S "/tmp/.X11-unix/X$DISPLAY_NUMBER" ||
    fail "Xvfb did not start: $(cat "$OUT/xvfb.log")"
}

draw_window() {
  export DISPLAY=":$DISPLAY_NUMBER" GDK_BACKEND=x11
  # shellcheck disable=SC2016 # $$ is the shell's that becomes the window
  dbus-run-session -- sh -c 'echo $$ > "$1" && exec fermix-desktop' sh "$WINDOW_PID_FILE" \
    > "$OUT/window.log" 2>&1 &
  wait_for "$WINDOW_WAIT_SECONDS" window_mapped ||
    fail "the window did not map in ${WINDOW_WAIT_SECONDS}s: $(tail -n 20 "$OUT/window.log")"
  sleep "$SETTLE_SECONDS"
  alive "$WINDOW_PID_FILE" || fail "the window exited after it mapped: $(tail -n 20 "$OUT/window.log")"
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

draw_on_x11() {
  start_display
  draw_window
  capture_screen
  stop "$WINDOW_PID_FILE"
  stop "$XVFB_PID_FILE"
}

# One capture of Weston's output by weston-screenshooter, moved to <file>. A screenshooter the
# compositor refuses says so and still exits 0, so the capture is the one file it wrote, or none.
wayland_capture() {
  local file="$1"
  local -a shots
  rm -rf "$SHOTS"
  mkdir -p "$SHOTS"
  (cd "$SHOTS" && WAYLAND_DISPLAY="$WAYLAND_SOCKET" weston-screenshooter) >> "$OUT/weston-screenshooter.log" 2>&1 ||
    return 1
  shots=("$SHOTS"/wayland-screenshot-*.png)
  [ "${#shots[@]}" -eq 1 ] && [ -f "${shots[0]}" ] || return 1
  mv -f -- "${shots[0]}" "$file"
}

# Weston's output is no longer what it showed before the window opened.
wayland_drawn() {
  wayland_capture "$WAYLAND_NOW" || return 1
  [ "$(sha256sum < "$WAYLAND_NOW")" != "$(sha256sum < "$OUT/wayland-empty.png")" ]
}

# The libwayland-client that <maps>, a window's /proc/<pid>/maps, shows loaded: the private
# prefix's, and no other. Prints its path.
check_wayland_client() {
  local loaded path
  loaded="$(sed -n 's,^.* \(/[^ ]*/libwayland-client\.so[^ /]*\)$,\1,p' "$1" | sort -u)"
  [ -n "$loaded" ] || fail "the window has no libwayland-client loaded"
  while read -r path; do
    [[ "$path" == "$PREFIX/lib/"* ]] || fail "the window loads $path, not libwayland-client from $PREFIX/lib"
  done <<< "$loaded"
  printf '%s\n' "$loaded"
}

# A headless Weston on its software renderer, with the kiosk shell, which gives the window the
# whole output. --debug lets weston-screenshooter capture it: without it the compositor refuses.
start_weston() {
  weston --backend=headless --renderer=pixman --shell=kiosk --width="${SCREEN%x*}" --height="${SCREEN#*x}" \
    --socket="$WAYLAND_SOCKET" --idle-time=0 --debug > "$OUT/weston.log" 2>&1 &
  echo "$!" > "$WESTON_PID_FILE"
  wait_for "$DISPLAY_WAIT_SECONDS" test -S "$XDG_RUNTIME_DIR/$WAYLAND_SOCKET" ||
    fail "Weston did not start: $(tail -n 20 "$OUT/weston.log")"
  wayland_capture "$OUT/wayland-empty.png" ||
    fail "weston-screenshooter cannot capture the empty output: $(tail -n 5 "$OUT/weston-screenshooter.log")"
}

# The window on Weston, with no DISPLAY, so a window that cannot reach the compositor exits rather
# than falling back to X11; captured once it has drawn and settled, its libwayland-client checked.
draw_on_wayland() {
  local client
  start_weston
  # shellcheck disable=SC2016 # $$ is the shell's that becomes the window
  env -u DISPLAY GDK_BACKEND=wayland WAYLAND_DISPLAY="$WAYLAND_SOCKET" \
    dbus-run-session -- sh -c 'echo $$ > "$1" && exec fermix-desktop' sh "$WAYLAND_WINDOW_PID_FILE" \
    > "$OUT/wayland-window.log" 2>&1 &
  wait_for "$WINDOW_WAIT_SECONDS" wayland_drawn ||
    fail "the window did not draw on Weston in ${WINDOW_WAIT_SECONDS}s: $(tail -n 20 "$OUT/wayland-window.log")"
  rm -f "$WAYLAND_NOW"
  sleep "$SETTLE_SECONDS"
  alive "$WAYLAND_WINDOW_PID_FILE" ||
    fail "the window exited after it drew on Weston: $(tail -n 20 "$OUT/wayland-window.log")"
  client="$(check_wayland_client "/proc/$(cat "$WAYLAND_WINDOW_PID_FILE")/maps")"
  printf '%s\n' "$client" > "$OUT/wayland-client.txt"
  wayland_capture "$OUT/wayland-screenshot.png" ||
    fail "weston-screenshooter cannot capture the window: $(tail -n 5 "$OUT/weston-screenshooter.log")"
  stop "$WAYLAND_WINDOW_PID_FILE"
  stop "$WESTON_PID_FILE"
  log "the window drew on Weston through $client; wayland-screenshot.png and wayland-empty.png"
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

# The init system the upgrade runs under, installed, then run as the container's first process in
# this one's place. Docker mounts the container's own cgroup namespace read-only, and systemd makes
# its scopes there, so it is remounted writable first, which the container's CAP_SYS_ADMIN allows.
# It does not return.
boot_systemd() {
  local family="$1" log="$OUT/boot.log"
  case "$family" in deb | rpm) ;; *) fail "the family is deb or rpm, not '$family'" ;; esac
  mkdir -p "$OUT"
  case "$family" in
    deb)
      export DEBIAN_FRONTEND=noninteractive
      apt-get -o Acquire::Retries=3 update > "$log" 2>&1 || fail "apt-get update failed: $(tail -n 20 "$log")"
      apt-get -o Acquire::Retries=3 install -y --no-install-recommends systemd systemd-sysv dbus \
        dbus-user-session libpam-systemd >> "$log" 2>&1 || fail "apt-get cannot install systemd: $(tail -n 20 "$log")"
      ;;
    rpm)
      dnf install -y --setopt=install_weak_deps=False systemd systemd-pam dbus-broker shadow-utils util-linux \
        > "$log" 2>&1 ||
        fail "dnf cannot install systemd: $(tail -n 20 "$log")"
      ;;
  esac
  mount -o remount,rw /sys/fs/cgroup ||
    fail "cannot remount the container's cgroup namespace writable: it needs CAP_SYS_ADMIN, not AppArmor's default"
  log "installed systemd; it runs next, as the container's first process"
  exec /usr/lib/systemd/systemd
}

# <command...> as the account, in its home, beside its user manager.
as_account() {
  local uid
  uid="$(id -u "$ACCOUNT")"
  (cd "$ACCOUNT_HOME" && runuser -u "$ACCOUNT" -- env HOME="$ACCOUNT_HOME" XDG_RUNTIME_DIR="/run/user/$uid" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" "$@")
}

# The account, lingering, with fermix.service enabled in its user manager and answering, as
# `fermix service install` leaves it for a person.
start_service() {
  local uid
  useradd --create-home --home-dir "$ACCOUNT_HOME" "$ACCOUNT" || fail "useradd cannot create the account $ACCOUNT"
  loginctl enable-linger "$ACCOUNT" || fail "loginctl cannot make $ACCOUNT linger"
  uid="$(id -u "$ACCOUNT")"
  wait_for "$USER_MANAGER_WAIT_SECONDS" test -S "/run/user/$uid/systemd/private" ||
    fail "the user manager of $ACCOUNT did not start in ${USER_MANAGER_WAIT_SECONDS}s"
  as_account fermix service install > "$OUT/service-install.log" 2>&1 ||
    fail "fermix service install failed: $(tail -n 20 "$OUT/service-install.log")"
  log "fermix service install enabled fermix.service for $ACCOUNT: see service-install.log"
}

# Prints the pid `fermix status` names when it names the engine <engine> running, and fails
# otherwise. Its answer is kept in $STATUS_ANSWER.
status_names() {
  local engine="$1" answer running
  answer="$(as_account fermix status 2>&1)" || { printf '%s\n' "$answer" > "$STATUS_ANSWER"; return 1; }
  printf '%s\n' "$answer" > "$STATUS_ANSWER"
  running="$(sed -n 's/^fermix: running (pid \([0-9][0-9]*\), version \([^,]*\), .*/\1 \2/p' <<< "$answer")"
  [ -n "$running" ] && [ "${running#* }" = "$engine" ] || return 1
  echo "${running%% *}"
}

# <engine> <name>: the pid of the engine <engine> once `fermix status` names it, with that answer
# kept in <name> under the output. Each try is one `fermix status`, a second apart.
engine_pid() {
  local engine="$1" name="$2" pid
  pid="$(wait_for "$ENGINE_WAIT_SECONDS" status_names "$engine")" ||
    fail "fermix status did not name the engine $engine in $ENGINE_WAIT_SECONDS tries: $(cat "$STATUS_ANSWER")"
  cp -- "$STATUS_ANSWER" "$OUT/$name"
  echo "$pid"
}

restart_service() {
  as_account systemctl --user daemon-reload || fail "systemctl --user daemon-reload failed for $ACCOUNT"
  as_account systemctl --user restart fermix.service || fail "systemctl --user restart fermix.service failed"
  log "reloaded $ACCOUNT's user manager and restarted fermix.service"
}

# What the account's user manager and engine logged, into account.journal.
save_journal() {
  local uid
  # Before the account exists there is nothing of its to keep.
  uid="$(id -u "$ACCOUNT" 2> /dev/null)" || return 0
  journalctl --no-pager "_UID=$uid" > "$OUT/account.journal" 2>&1 || log "journalctl cannot read $ACCOUNT's journal"
}

upgrade_smoke() {
  local family="$1" previous="$2" previous_digest="$3" package="$4" digest="$5" before after old new
  mkdir -p "$OUT"
  trap save_journal EXIT
  install_package "$family" "$previous" "$previous_digest" install.log
  before="$(provided_engine "$family")"
  check_engine "$before" fermix-version-previous.txt
  start_service
  old="$(engine_pid "$before" status-previous.txt)"
  log "fermix-desktop $(installed_version "$family") runs the engine $before as pid $old"
  install_package "$family" "$package" "$digest" upgrade.log
  [ "$(installed_version "$family")" = "$(package_version "$family" "$package")" ] ||
    fail "fermix-desktop $(installed_version "$family") is installed, not $(package_version "$family" "$package")"
  after="$(provided_engine "$family")"
  check_engine "$after" fermix-version.txt
  [ "$(engine_pid "$before" status-upgraded.txt)" = "$old" ] ||
    fail "the engine $before is not still running as pid $old after the upgrade"
  log "upgraded to fermix-desktop $(installed_version "$family"); the engine $before still runs as pid $old"
  restart_service
  new="$(engine_pid "$after" status-restarted.txt)"
  [ "$new" != "$old" ] || fail "fermix.service still runs pid $old after the restart"
  log "the engine $after runs as pid $new"
  check_libraries
  check_control
  save_journal
  trap - EXIT
  write_sums
}

fresh_smoke() {
  local family="$1" package="$2" digest="$3" displays="$4" engine display
  mkdir -p "$OUT"
  install_package "$family" "$package" "$digest" install.log
  engine="$(provided_engine "$family")"
  log "fermix-desktop provides fermix $engine"
  check_engine "$engine" fermix-version.txt
  check_libraries
  check_control
  install_display_tools "$family" "$displays"
  prepare_session
  for display in $displays; do
    "draw_on_$display"
  done
  write_sums
}

usage() {
  fail "usage: smoke_in_container.sh --family <deb|rpm> --package <file> --sha256 <hex>" \
    "(--displays \"<x11 | wayland | x11 wayland>\" | --upgrade-from <file> --previous-sha256 <hex>)," \
    "or smoke_in_container.sh --boot <deb|rpm>"
}

main() {
  local family="" package="" digest="" displays="" previous="" previous_digest="" display
  local -a words
  if [ "${1:-}" = --boot ]; then
    [ $# -eq 2 ] || usage
    boot_systemd "$2"
  fi
  while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || usage
    case "$1" in
      --family) family="$2" ;; --package) package="$2" ;; --sha256) digest="$2" ;;
      --displays) displays="$2" ;; --upgrade-from) previous="$2" ;; --previous-sha256) previous_digest="$2" ;;
      *) usage ;;
    esac
    shift 2
  done
  case "$family" in deb | rpm) ;; *) fail "the family is deb or rpm, not '$family'" ;; esac
  [ -n "$package" ] && [ -n "$digest" ] || fail "every flag needs a value"
  if [ -n "$previous$previous_digest" ]; then
    [ -n "$previous" ] && [ -n "$previous_digest" ] && [ -z "$displays" ] ||
      fail "an upgrade takes --upgrade-from and --previous-sha256, and draws no window"
    upgrade_smoke "$family" "$previous" "$previous_digest" "$package" "$digest"
    return 0
  fi
  read -r -a words <<< "$displays"
  [ "${#words[@]}" -gt 0 ] || fail "every flag needs a value"
  for display in "${words[@]}"; do
    case "$display" in x11 | wayland) ;; *) fail "the displays are x11 and wayland, not '$display'" ;; esac
  done
  fresh_smoke "$family" "$package" "$digest" "${words[*]}"
}

# Run, not sourced: smoke_in_container_test.sh sources this file for its functions.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
