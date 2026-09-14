#!/usr/bin/env bash
#
# Install both packages on a machine that has a systemd, and see the window open
# against a real daemon (M38 section 13.1).
#
# Everything before this gate reads files. This one installs what was built, on a
# host whose package manager resolves the declared relations out of the
# distribution's own archive, under an ordinary account whose own `systemd --user`
# runs the engine. It is the first and only place the two packages meet.
#
# What it proves:
#
#   1. `apt-get install ./engine.deb ./desktop.deb` succeeds, which means every
#      relation the desktop package declares is satisfiable from the archive and
#      the exact-version relation on the engine is satisfied by the engine
#      package beside it;
#   2. the desktop package installs exactly the files it claims, at the paths the
#      desktop reads them from;
#   3. `fermix service install --json` brings the engine up under this account's
#      own user manager, and `fermix service status --json` then reports the
#      installed and running builds as aligned;
#   4. `fermix-desktop --version` answers from the packaged binary;
#   5. the window opens against that real daemon, with no fixture home and no
#      fake command line, and the window the display server sees carries the
#      application identity as its class, which is the one place that string is
#      checked against a running window rather than against a file.
#
# What it deliberately does not prove: anything that needs a desktop. A portal
# backend, a notification daemon, a shell that owns a tray watcher, an accent
# colour, a person. Those are the hand-verified gates of M38 section 13.2, and
# docs/ACCEPTANCE_RUNBOOK.md carries them.
#
# One thing is arranged rather than exercised, and it is worth saying: linger is
# granted to the account by root before the engine's own install runs. The engine
# skips its `loginctl enable-linger` when linger is already on, so this gate does
# not exercise the polkit hop. The engine's own suite covers that path in both of
# its shapes; what is being proven here is packaging.
#
# Usage: install_smoke.sh <engine.deb> <desktop.deb>
#   Either may be given as `-` to say it does not exist: the gate then runs
#   everything that does not need it and reports exactly what it could not run.
set -euo pipefail

USAGE="usage: install_smoke.sh <engine.deb|-> <desktop.deb>"

ENGINE_DEB="${1:?$USAGE}"
DESKTOP_DEB="${2:?$USAGE}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${FERMIX_SMOKE_IMAGE:-fermix-desktop-smoke}"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.smoke"
EVIDENCE="$ROOT_DIR/packaging/out/smoke"
CONTAINER="fermix-desktop-smoke-$$"
APP_ID="io.tezra.Fermix"
HOME_PATH="/home/test/fermix home"

skipped=()

fail() {
  echo "install_smoke: $*" >&2
  exit 1
}

step() {
  echo
  echo "install_smoke: == $*"
}

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

command -v docker >/dev/null 2>&1 || fail "docker is not installed"
[ -f "$DOCKERFILE" ] || fail "no smoke container at $DOCKERFILE"
[ -f "$DESKTOP_DEB" ] || fail "no desktop package at $DESKTOP_DEB"

if [ "$ENGINE_DEB" = "-" ]; then
  skipped+=("the engine package was not given, so nothing that needs a daemon ran")
  ENGINE_DEB=""
else
  [ -f "$ENGINE_DEB" ] || fail "no engine package at $ENGINE_DEB"
fi

# ---- the machine -----------------------------------------------------------

step "the machine"
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  docker build -f "$DOCKERFILE" -t "$IMAGE" "$ROOT_DIR/packaging/docker"
fi

# systemd as process one needs the cgroup filesystem and a writable /run. This is
# a throwaway container on a developer's machine or a CI runner, and it is torn
# down on every exit path above.
docker run -d --name "$CONTAINER" \
  --privileged \
  --cgroupns=host \
  --tmpfs /run --tmpfs /run/lock --tmpfs /tmp \
  -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
  "$IMAGE" >/dev/null

inside() {
  docker exec "$CONTAINER" "$@"
}

# As the ordinary account, with the runtime directory its user manager owns.
as_test() {
  docker exec \
    -u test \
    -e "XDG_RUNTIME_DIR=/run/user/$(inside id -u test | tr -d '\r')" \
    -e HOME=/home/test \
    "$CONTAINER" "$@"
}

echo -n "  waiting for systemd"
for _ in $(seq 1 60); do
  state="$(inside systemctl is-system-running 2>/dev/null || true)"
  case "$state" in
    running | degraded) break ;;
  esac
  echo -n "."
  sleep 1
done
echo
case "$(inside systemctl is-system-running 2>/dev/null || true)" in
  running | degraded) echo "  systemd is up" ;;
  *) fail "systemd never came up in the container" ;;
esac

# ---- the install -----------------------------------------------------------

step "the install"
inside mkdir -p /packages
docker cp "$DESKTOP_DEB" "$CONTAINER:/packages/$(basename "$DESKTOP_DEB")"
[ -z "$ENGINE_DEB" ] ||
  docker cp "$ENGINE_DEB" "$CONTAINER:/packages/$(basename "$ENGINE_DEB")"

inside apt-get update -qq

DESKTOP_VERSION="$(inside dpkg-deb --field "/packages/$(basename "$DESKTOP_DEB")" Version | tr -d '\r')"
echo "  fermix-desktop $DESKTOP_VERSION"

# The two are a pair or they are not, and saying so here is kinder than letting
# apt's solver say it in the shape of a dependency graph. This is also the one
# place the exact-version relation can be seen doing its job: a mismatched pair
# is refused rather than half installed.
if [ -n "$ENGINE_DEB" ]; then
  ENGINE_VERSION="$(inside dpkg-deb --field "/packages/$(basename "$ENGINE_DEB")" Version | tr -d '\r')"
  echo "  fermix $ENGINE_VERSION"
  [ "$ENGINE_VERSION" = "$DESKTOP_VERSION" ] ||
    fail "the engine package is $ENGINE_VERSION and the desktop package is $DESKTOP_VERSION; the desktop package declares fermix (= $DESKTOP_VERSION), so these two are not a pair and apt will refuse them"
fi

# One command for both packages, which is what an operator runs and what makes
# the exact-version relation between them meaningful: apt resolves the desktop
# package's `fermix (= <version>)` against the engine package in the same
# transaction, and refuses the pair rather than half installing one.
if [ -n "$ENGINE_DEB" ]; then
  inside apt-get install -y -qq \
    "/packages/$(basename "$ENGINE_DEB")" "/packages/$(basename "$DESKTOP_DEB")" ||
    fail "the two packages did not install together"
else
  # Without the engine, the exact-version relation cannot be satisfied at all,
  # and apt is right to refuse: this package is half of a pair. So the other
  # relations are installed by name first, read out of the package's own control
  # data rather than listed again here, and then the package is forced in with
  # that one relation unmet. `--fix-broken` is deliberately not run afterwards,
  # because the way apt fixes an unsatisfiable relation is by taking the package
  # away again.
  #
  # Installing the declared relations by name is itself worth something: it is
  # the proof that every one of them exists in the distribution's archive under
  # the name the package writes down.
  relations="$(inside dpkg-deb --field "/packages/$(basename "$DESKTOP_DEB")" Depends |
    tr ',' '\n' | sed 's/(.*)//' | tr -d ' \r' | grep -v '^fermix$' | grep -v '^$')"
  echo "  installing the relations this package declares, except the engine:"
  printf '    %s\n' $relations
  # shellcheck disable=SC2086
  inside apt-get install -y -qq $relations ||
    fail "the package declares a relation the archive does not carry under that name"
  inside dpkg --force-depends -i "/packages/$(basename "$DESKTOP_DEB")" ||
    fail "the desktop package did not unpack"
  skipped+=("the engine was not installed, so the exact-version relation between the two packages was not proven, and neither was anything that needs a daemon")
fi
echo "  installed"

step "what the package installed"
inside dpkg -L fermix-desktop | sort | sed 's/^/    /'
for path in \
  /usr/bin/fermix-desktop \
  "/usr/share/applications/$APP_ID.desktop" \
  "/usr/share/metainfo/$APP_ID.metainfo.xml" \
  "/usr/share/dbus-1/services/$APP_ID.service" \
  "/usr/lib/systemd/user/app-$APP_ID.service" \
  "/usr/share/icons/hicolor/256x256/apps/$APP_ID.png" \
  "/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
  /usr/share/fermix-desktop/build.json \
  /usr/share/doc/fermix-desktop/copyright; do
  inside test -f "$path" || fail "the package does not install $path"
done
echo "  every path the package claims is there"

# The window compares the id compiled into it with the one in this file, so the
# two have to be one value. A package that ships one and not the other is
# silently unknown rather than wrong, which is why it is checked here.
#
# Read with sed rather than with a JSON parser, because this is a machine that a
# person could have rather than a build image: it carries no python, and adding
# one to read four fields would be weight the gate does not need.
BUILD_ID="$(inside sed -n 's/.*"build_id"[^"]*"\([^"]*\)".*/\1/p' /usr/share/fermix-desktop/build.json | tr -d '\r')"
case "$BUILD_ID" in
  "" | *[[:space:]]*) fail "the installed manifest names no build id: '$BUILD_ID'" ;;
esac
echo "  the installed manifest names build $BUILD_ID"

step "the packaged binary says what it is"
VERSION_LINE="$(inside /usr/bin/fermix-desktop --version)"
echo "  $VERSION_LINE"
case "$VERSION_LINE" in
  *"(build $BUILD_ID)"*) echo "  the binary and the manifest name one build" ;;
  *) fail "the binary reports '$VERSION_LINE' and the manifest names $BUILD_ID" ;;
esac

# ---- the engine, under this account's own systemd --------------------------

DAEMON_RUNNING=0
if [ -n "$ENGINE_DEB" ]; then
  step "the engine, under this account's own user manager"

  inside loginctl enable-linger test
  echo -n "  waiting for the user manager"
  for _ in $(seq 1 30); do
    if inside systemctl is-active "user@$(inside id -u test | tr -d '\r').service" >/dev/null 2>&1; then
      break
    fi
    echo -n "."
    sleep 1
  done
  echo
  inside systemctl is-active "user@$(inside id -u test | tr -d '\r').service" >/dev/null ||
    fail "this account has no user manager, so there is nowhere to install a user unit"
  echo "  the user manager is up"

  # A home with a space in it, because that is the path shape that breaks a
  # renderer that forgot to quote (M38 section 13.1 gate 29).
  INSTALL_JSON="$(as_test fermix service install --json --home "$HOME_PATH")" ||
    fail "fermix service install refused: $INSTALL_JSON"
  echo "  install: $INSTALL_JSON"
  case "$INSTALL_JSON" in
    *'"ok":true'*) ;;
    *) fail "fermix service install did not report ok" ;;
  esac

  STATUS_JSON="$(as_test fermix service status --json)" ||
    fail "fermix service status refused: $STATUS_JSON"
  echo "  status: $STATUS_JSON"
  case "$STATUS_JSON" in
    *'"alignment":"aligned"'*) echo "  the installed engine and the running engine are aligned" ;;
    *) fail "fermix service status does not report the two builds as aligned" ;;
  esac

  # What `service install` promised: enabled for the next login and running now.
  # A smoke that accepted a status saying otherwise would not notice an engine
  # whose install reports success and leaves nothing running, which is a state
  # this gate has already seen on one systemd version.
  case "$STATUS_JSON" in
    *'"enabled":true'*) ;;
    *) fail "the service is installed and the status does not report it as enabled" ;;
  esac
  case "$STATUS_JSON" in
    *'"active":true'*) echo "  the service is enabled and running" ;;
    *) fail "the service is installed and the status does not report it as active" ;;
  esac
  DAEMON_RUNNING=1
fi

# ---- the window ------------------------------------------------------------

step "the window, on a display"
mkdir -p "$EVIDENCE"

if [ "$DAEMON_RUNNING" = "1" ]; then
  echo "  against the daemon this run installed"
else
  echo "  with no daemon to speak to, which is a state the window draws"
fi

# The release binary has no capture mode: that is a debug build's, and a package
# ships a release build. So the picture is taken of the display rather than by
# the application, which also proves the one thing the application's own renderer
# could not: the class the display server sees on the window, which is the sixth
# of the six places the application identity lives and the only one that can be
# checked against a running window rather than against a file.
#
# A session bus of this run's own, because a GtkApplication takes its name on the
# bus and a window that could not register is a window that never draws. M38
# section 13.1 names `dbus-run-session` for exactly this. Nothing on that bus
# outlives the check.
#
# The script is written here and copied in rather than quoted through two shells,
# because a nested quote is how a check like this silently stops checking.
# Everything this step writes lives in /evidence, which is an ordinary
# directory: /tmp inside the container is a tmpfs, and `docker cp` reads and
# writes the image layer underneath a tmpfs rather than the live mount, so a
# file copied either way through /tmp is a file that is not there.
inside mkdir -p /evidence
# Owned by the account that runs the window, because that account is the one that
# writes the picture.
inside chown test:test /evidence
WINDOW_SCRIPT="$(mktemp "${TMPDIR:-/tmp}/fermix-window.XXXXXX")"
cat > "$WINDOW_SCRIPT" <<EOF
#!/bin/bash
# Open the window on a display of this run's own, wait for the display server to
# report a window carrying the application's class, photograph the screen, and
# stop.
set -uo pipefail

application_id="$APP_ID"
EOF
cat >> "$WINDOW_SCRIPT" <<'EOF'

/usr/bin/fermix-desktop &
application=$!

found=
for _ in $(seq 1 40); do
  if xdotool search --class "$application_id" >/dev/null 2>&1; then
    found=yes
    break
  fi
  sleep 0.5
done

# A window exists; give the first read of the daemon and the toolkit's own
# transition time to land before the picture is taken.
sleep 3

import -window root /evidence/home_running.png
xdotool search --class "$application_id" getwindowname %@ > /evidence/window.txt 2>/dev/null
xprop -root _NET_CLIENT_LIST > /evidence/clients.txt 2>/dev/null

kill "$application" 2>/dev/null
wait "$application" 2>/dev/null

[ -n "$found" ]
EOF
chmod +x "$WINDOW_SCRIPT"
docker cp "$WINDOW_SCRIPT" "$CONTAINER:/evidence/window_check.sh" >/dev/null
inside chmod 0755 /evidence/window_check.sh
rm -f "$WINDOW_SCRIPT"

set +e
docker exec -u test \
  -e "XDG_RUNTIME_DIR=/run/user/$(inside id -u test | tr -d '\r')" \
  -e HOME=/home/test \
  "$CONTAINER" \
  dbus-run-session -- \
  xvfb-run -a --server-args="-screen 0 1280x800x24" /evidence/window_check.sh
WINDOW_STATUS=$?
set -e

[ "$WINDOW_STATUS" -eq 0 ] ||
  fail "no window carrying the class $APP_ID appeared within twenty seconds"
echo "  a window carrying the class $APP_ID appeared"
echo "  its title: $(inside cat /evidence/window.txt 2>/dev/null || echo unknown)"

docker cp "$CONTAINER:/evidence/home_running.png" "$EVIDENCE/home_running.png" >/dev/null 2>&1 ||
  fail "the window was drawn and no picture of it could be taken"
echo "  $EVIDENCE/home_running.png"

# ---- what ran, and what did not --------------------------------------------

step "what ran"
echo "  the two packages installed, the file list is what the package claims,"
echo "  the binary and the manifest name one build, and a window with the"
echo "  application's own class was drawn on a real display."

if [ "${#skipped[@]}" -gt 0 ]; then
  echo
  echo "install_smoke: what could not run"
  for line in "${skipped[@]}"; do
    echo "  $line"
  done
fi

echo
echo "install_smoke: ok"
