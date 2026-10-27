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
# which says where an rpm image's test tools come from; use_almalinux_origin, which points
# AlmaLinux's repositories at AlmaLinux's own address; provided_engine, which reads the engine
# version `fermix --version` has to answer with from what the installed package provides;
# status_names, which reads the engine `fermix status` names running during an upgrade;
# save_journal, which keeps the account's journal and the container's; start_service, which gives
# root owner read on the shadow file before the linger starts the user manager; check_helper, which
# holds the engine's payload to its fermix-iroh helper and that helper's notices, and the package's
# file list to the notices' doc copy; and the flags the script accepts.
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

echo "smoke_in_container_test: AlmaLinux reads its own repository, not the mirrors its list picks"
# Two sections of almalinux-repos' almalinux-baseos.repo as AlmaLinux 10 ships it.
mkdir -p "$work/repos" "$work/repos-fedora" "$work/repos-bare"
cat > "$work/baseos.repo" << 'REPO'
[baseos]
name=AlmaLinux $releasever - BaseOS
mirrorlist=https://mirrors.almalinux.org/mirrorlist/$releasever/baseos
# baseurl=https://repo.almalinux.org/almalinux/$releasever/BaseOS/$basearch/os/
enabled=1

[baseos-source]
name=AlmaLinux $releasever - BaseOS - Source
mirrorlist=https://mirrors.almalinux.org/mirrorlist/$releasever/baseos-source
# baseurl=https://vault.almalinux.org/$releasever/BaseOS/Source/
enabled=0
REPO
cp "$work/baseos.repo" "$work/repos/almalinux-baseos.repo"
cp "$work/baseos.repo" "$work/repos-fedora/almalinux-baseos.repo"
sourced use_almalinux_origin "$work/os-almalinux" "$work/repos"
! grep -q '^mirrorlist=' "$work/repos/almalinux-baseos.repo" || fail "AlmaLinux still reads a mirror list"
# shellcheck disable=SC2016 # dnf's own variables, written as the file holds them
grep -qx 'baseurl=https://repo.almalinux.org/almalinux/$releasever/BaseOS/$basearch/os/' \
  "$work/repos/almalinux-baseos.repo" || fail "AlmaLinux's BaseOS does not read repo.almalinux.org"
sourced use_almalinux_origin "$work/os-almalinux" "$work/repos"
[ "$(grep -c '^baseurl=' "$work/repos/almalinux-baseos.repo")" = 2 ] || fail "a second run changed the file again"
sourced use_almalinux_origin "$work/os-fedora" "$work/repos-fedora"
cmp -s "$work/baseos.repo" "$work/repos-fedora/almalinux-baseos.repo" || fail "an image that is not AlmaLinux lost its mirrors"
echo "  ok: every AlmaLinux section reads the address its file names, twice the same, and Fedora keeps its mirrors"
grep -v 'vault' "$work/baseos.repo" > "$work/repos-bare/almalinux-baseos.repo"
expect_refusal "an AlmaLinux section with no address of its own" \
  "has a repository with no baseurl to read from in place of its mirror list" \
  sourced use_almalinux_origin "$work/os-almalinux" "$work/repos-bare"
expect_refusal "an AlmaLinux image with no repository file" "holds no AlmaLinux repository file" \
  sourced use_almalinux_origin "$work/os-almalinux" "$work/repos-none"

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

# A stand-in runuser that runs what follows its --, an id that names the account 1001, and a fermix
# whose `status` prints $STATUS_FILE and exits with $STATUS_EXIT.
cat > "$work/bin/runuser" << 'STUB'
#!/usr/bin/env bash
while [ "$1" != -- ]; do shift; done
shift
exec "$@"
STUB
printf '#!/bin/sh\necho 1001\n' > "$work/bin/id"
cat > "$work/bin/fermix" << 'STUB'
#!/usr/bin/env bash
[ "$*" = status ] || { echo "stand-in fermix was asked: $*" >&2; exit 2; }
cat "$STATUS_FILE"
exit "$STATUS_EXIT"
STUB
chmod 0755 "$work/bin/runuser" "$work/bin/id" "$work/bin/fermix"
export STATUS_FILE="$work/status"

# <function> <args...> from the script, as sourced() runs them, with the account's home and the
# kept answer of `fermix status` under the work directory.
sourced_as_account() {
  (
    # shellcheck source=desktop/packaging/smoke_in_container.sh
    source "$here/smoke_in_container.sh"
    trap - ERR
    ACCOUNT_HOME="$work" STATUS_ANSWER="$work/answer"
    PATH="$work/bin:$PATH" "$@"
  )
}

# <answer> <exit> <engine>: the pid status_names prints when `fermix status` gives that answer, or
# "refused" when it finds no engine <engine> running.
status_pid() {
  printf '%b\n' "$1" > "$STATUS_FILE"
  STATUS_EXIT="$2" sourced_as_account status_names "$3" || echo refused
}

echo "smoke_in_container_test: the engine that fermix status names"
running='fermix: running (pid 4193, version 0.14.0, up 14s, protocol v2)'
[ "$(status_pid "$running" 0 0.14.0)" = 4193 ] || fail "status_names did not read pid 4193 of 0.14.0"
skew="[l] Uninstalled older version (v0.14.0)\n$running\nwarning: daemon is running 0.14.0 but the installed binary is 0.15.0"
[ "$(status_pid "$skew" 0 0.14.0)" = 4193 ] || fail "status_names did not read the engine beside a skew warning"
grep -qF 'warning: daemon is running 0.14.0' "$work/answer" || fail "status_names did not keep the whole answer"
echo "  ok: the pid of the engine it names, alone and beside the engine's own notes, the answer kept"
[ "$(status_pid "$running" 0 0.15.0)" = refused ] || fail "status_names took 0.14.0 for 0.15.0"
[ "$(status_pid "$running" 0 0.14)" = refused ] || fail "status_names took 0.14.0 for 0.14"
[ "$(status_pid 'fermix: not running' 3 0.14.0)" = refused ] || fail "status_names took a stopped engine for a running one"
grep -qxF 'fermix: not running' "$work/answer" || fail "status_names did not keep the answer of a stopped engine"
[ "$(status_pid 'fermix: running (pid ?, version 0.14.0, up 1s, protocol v2)' 0 0.14.0)" = refused ] ||
  fail "status_names took an engine with no pid for a running one"
echo "  ok: another version, a stopped engine and an engine with no pid are not the engine it waits for"

# A stand-in journalctl that prints what it was asked.
printf '#!/bin/sh\necho "journalctl $*"\n' > "$work/bin/journalctl"
chmod 0755 "$work/bin/journalctl"

echo "smoke_in_container_test: the journals an upgrade keeps"
mkdir -p "$work/out"
sourced save_journal
grep -qxF 'journalctl --no-pager _UID=1001' "$work/out/account.journal" ||
  fail "save_journal did not keep the account's journal: $(cat "$work/out/account.journal")"
[ -f "$work/out/system.journal" ] || fail "save_journal did not keep the system journal"
grep -qxF 'journalctl --no-pager' "$work/out/system.journal" ||
  fail "save_journal did not keep the container's whole journal: $(cat "$work/out/system.journal")"
echo "  ok: the account's journal, and the container's whole journal, logind's and user@1001's among it"

# A stand-in useradd, and a loginctl that keeps the mode the shadow file has when it is asked to
# make the account linger, which starts its user manager, and then refuses.
printf '#!/bin/sh\nexit 0\n' > "$work/bin/useradd"
printf '#!/bin/sh\nstat -c %%a "%s" > "%s"\nexit 1\n' "$work/shadow" "$work/linger-saw" > "$work/bin/loginctl"
chmod 0755 "$work/bin/useradd" "$work/bin/loginctl"

echo "smoke_in_container_test: the shadow file root reads when the user manager starts"
install -m 0000 /dev/null "$work/shadow"
if (
  # shellcheck source=desktop/packaging/smoke_in_container.sh
  source "$here/smoke_in_container.sh"
  trap - ERR
  SHADOW="$work/shadow" ACCOUNT_HOME="$work" OUT="$work/out" PATH="$work/bin:$PATH" start_service
) 2> /dev/null; then
  fail "start_service went on past a loginctl that refused"
fi
[ -f "$work/linger-saw" ] || fail "start_service never asked loginctl to make the account linger"
[ "$(cat "$work/linger-saw")" = 400 ] ||
  fail "the shadow file was mode $(cat "$work/linger-saw"), not 400, when the user manager started"
echo "  ok: Fedora's mode 0000 is 0400 by then, which root reads with no capability"

# A stand-in fermix whose --version extracts a payload as Burrito does, under
# $FERMIX_LINUX_PACKAGE_INSTALL_DIR: the engine $PAYLOAD_ENGINE's iroh_helper folder, in the shape
# $PAYLOAD_SHAPE names. A stand-in dpkg and rpm list fermix-desktop's files from $LIST_FILE.
mkdir -p "$work/helper-bin"
cat > "$work/helper-bin/fermix" << 'STUB'
#!/usr/bin/env bash
[ "$*" = --version ] || { echo "stand-in fermix was asked: $*" >&2; exit 2; }
[ "$PAYLOAD_SHAPE" != failing ] || { echo "payload extraction failed" >&2; exit 1; }
dir="$FERMIX_LINUX_PACKAGE_INSTALL_DIR/.burrito/fermix_linux_package_erts-16.3_$PAYLOAD_ENGINE"
dir="$dir/lib/fermix_core-$PAYLOAD_ENGINE/priv/iroh_helper"
mkdir -p "$dir/bin"
answer='fermix-iroh 0.2.2 (iroh 1.3.0)'
[ "$PAYLOAD_SHAPE" != wrong-answer ] || answer='erl: not a helper'
[ "$PAYLOAD_SHAPE" = no-helper ] || printf '#!/bin/sh\necho "%s"\n' "$answer" > "$dir/bin/fermix-iroh"
[ "$PAYLOAD_SHAPE" = no-helper ] || [ "$PAYLOAD_SHAPE" = not-executable ] || chmod 0755 "$dir/bin/fermix-iroh"
case "$PAYLOAD_SHAPE" in
  no-notices) ;;
  empty-notices) : > "$dir/THIRD-PARTY-NOTICES.txt" ;;
  *) echo 'attohttpc 0.30.1: Mozilla Public License 2.0' > "$dir/THIRD-PARTY-NOTICES.txt" ;;
esac
echo "fermix $PAYLOAD_ENGINE"
STUB
cat > "$work/helper-bin/dpkg" << 'STUB'
#!/usr/bin/env bash
[ "$*" = '-L fermix-desktop' ] || { echo "stand-in dpkg was asked: $*" >&2; exit 2; }
cat "$LIST_FILE"
STUB
cat > "$work/helper-bin/rpm" << 'STUB'
#!/usr/bin/env bash
[ "$*" = '-ql fermix-desktop' ] || { echo "stand-in rpm was asked: $*" >&2; exit 2; }
cat "$LIST_FILE"
STUB
chmod 0755 "$work/helper-bin/fermix" "$work/helper-bin/dpkg" "$work/helper-bin/rpm"
export LIST_FILE="$work/list" PAYLOAD_ENGINE=0.17.0 PAYLOAD_SHAPE=whole
printf '%s\n' /usr/bin/fermix /usr/share/doc/fermix/fermix-iroh-THIRD-PARTY-NOTICES.txt > "$LIST_FILE"

# check_helper <args...> from the script, as sourced() runs it, with the stand-ins first on PATH and
# a payload folder of its own each time.
helper_check() {
  rm -rf "$work/payload" "$work/out"
  mkdir -p "$work/out"
  (
    # shellcheck source=desktop/packaging/smoke_in_container.sh
    source "$here/smoke_in_container.sh"
    trap - ERR
    PAYLOAD_DIR="$work/payload" OUT="$work/out"
    PATH="$work/helper-bin:$PATH" check_helper "$@"
  )
}

echo "smoke_in_container_test: the engine's payload holds its fermix-iroh helper and notices"
for family in deb rpm; do
  helper_check 0.17.0 "$family" 2> /dev/null || fail "the $family's whole payload was refused"
done
grep -qxF 'fermix-iroh 0.2.2 (iroh 1.3.0)' "$work/out/fermix-iroh-version.txt" ||
  fail "the helper's answer was not kept: $(cat "$work/out/fermix-iroh-version.txt" 2> /dev/null)"
echo "  ok: a payload with the helper and its notices, from a deb and from an rpm"
PAYLOAD_ENGINE=0.16.0 expect_refusal "the payload of another engine" \
  "the engine 0.17.0's payload holds 0 iroh_helper folders, not 1" helper_check 0.17.0 deb
PAYLOAD_SHAPE=failing expect_refusal "a payload fermix cannot extract" \
  "fermix --version failed to extract its payload" helper_check 0.17.0 deb
PAYLOAD_SHAPE=no-helper expect_refusal "a payload with no helper, as engine v0.16.0's" \
  "has no executable fermix-iroh" helper_check 0.17.0 deb
PAYLOAD_SHAPE=not-executable expect_refusal "a helper that cannot run" \
  "has no executable fermix-iroh" helper_check 0.17.0 deb
PAYLOAD_SHAPE=wrong-answer expect_refusal "a helper that answers as something else" \
  "fermix-iroh --version answers 'erl: not a helper'" helper_check 0.17.0 deb
PAYLOAD_SHAPE=no-notices expect_refusal "a helper with no notices" \
  "has no third-party notices beside its helper" helper_check 0.17.0 deb
PAYLOAD_SHAPE=empty-notices expect_refusal "a helper with empty notices" \
  "has no third-party notices beside its helper" helper_check 0.17.0 deb
printf '%s\n' /usr/bin/fermix > "$LIST_FILE"
for family in deb rpm; do
  expect_refusal "the $family's file list without the notices' doc copy" \
    "fermix-desktop does not install /usr/share/doc/fermix/fermix-iroh-THIRD-PARTY-NOTICES.txt" \
    helper_check 0.17.0 "$family"
done
rm "$LIST_FILE"
expect_refusal "a package database that cannot list the package" \
  "the rpm database cannot list fermix-desktop's files" helper_check 0.17.0 rpm

echo "smoke_in_container_test: the flags it accepts"
run_script() {
  bash "$here/smoke_in_container.sh" --family rpm --package /smoke/p.rpm --sha256 0 "$@"
}
expect_refusal "an upgrade with no previous sha256" "an upgrade takes --upgrade-from and --previous-sha256" \
  run_script --upgrade-from /smoke/q.rpm
expect_refusal "a previous sha256 with no upgrade" "an upgrade takes --upgrade-from and --previous-sha256" \
  run_script --previous-sha256 0
expect_refusal "an upgrade that draws a window" "and draws no window" \
  run_script --upgrade-from /smoke/q.rpm --previous-sha256 0 --displays x11
expect_refusal "a boot for another family" "the family is deb or rpm, not 'arch'" \
  bash "$here/smoke_in_container.sh" --boot arch
expect_refusal "a boot with a package" "usage" bash "$here/smoke_in_container.sh" --boot deb --package /smoke/p.deb
expect_refusal "a display it does not know" "the displays are x11 and wayland, not 'mir'" \
  run_script --displays mir
expect_refusal "one it knows beside one it does not" "not 'mir'" run_script --displays "x11 mir"
expect_refusal "no display" "every flag needs a value" run_script --displays ""
expect_refusal "no --displays" "every flag needs a value" run_script
expect_refusal "an engine version named by the caller: the package says it" "usage" \
  run_script --displays x11 --engine-version 0.14.0

echo "smoke_in_container_test: ok"
