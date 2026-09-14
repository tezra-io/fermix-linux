#!/usr/bin/env bash
#
# Render the reference states to PNGs, light and dark, against the fixture
# daemon.
#
# One fixture daemon and one application per scenario and per colour scheme:
# the scenario decides what the daemon answers, and the scheme is the
# environment the toolkit reads. The application itself filters the state list
# down to the states its scenario can show, so a run that produces fewer files
# than another is a scenario with fewer states rather than a failure.
#
# Usage:
#   scripts/capture.sh                 every scenario, both schemes
#   scripts/capture.sh doctor_failed   one scenario, both schemes
#   scripts/capture.sh --container     the same, inside the build container
#
# The captures that get reviewed are the container's: it is Linux, it carries
# the toolkit floor, and it has the icon theme a desktop has. A development Mac
# has no Adwaita icon theme at all, so a capture taken there is honest about
# layout and copy and silent about icons.
#
# The files land in docs/design/captures/ and every one of them is listed in
# docs/design/captures/INDEX.md with the facts a reviewer needs to read it.
# A capture that exists is not an accepted capture: the decision column is
# filled in by a person.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="${FERMIX_CRATE_DIR:-$ROOT_DIR/App/Fermix}"
OUT_DIR="${FERMIX_CAPTURE_DIR:-$ROOT_DIR/docs/design/captures}"
INDEX="$OUT_DIR/INDEX.md"

SCENARIOS=(default setup_required restart_pending not_running external_change unreadable \
           doctor_healthy doctor_failed logs_empty \
           integrations_states meetings_signed_out computer_states computer_wayland \
           onboarding_welcome onboarding_starting onboarding_linger_denied \
           onboarding_boot_failed onboarding_connect_ai onboarding_about_you \
           onboarding_refused_personalization onboarding_no_restart \
           onboarding_restart_needed onboarding_ready onboarding_skew)
SCHEMES=(prefer-light prefer-dark)

IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"
CACHE_VOLUME="${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}"
CONTAINER=0

fail() {
  echo "capture: $*" >&2
  exit 1
}

# The command line's own state for a scenario, so the unit the window reports
# and the daemon it is talking to agree with each other.
cli_state() {
  case "$1" in
    not_running) echo bound_disabled ;;
    restart_pending) echo pending_restart ;;
    # The Setup assistant's states are the command line's as much as the
    # daemon's: what the ladder shows is what `service install` and `restart`
    # answer, so each scenario names the state that answers it.
    onboarding_welcome) echo fresh ;;
    onboarding_starting) echo install_ok ;;
    onboarding_linger_denied) echo linger_denied ;;
    onboarding_boot_failed) echo install_activation_timeout ;;
    onboarding_restart_needed) echo restart_refused ;;
    onboarding_skew) echo pending_restart ;;
    *) echo active_aligned ;;
  esac
}

# The session type a scenario is taken in. One state of the Computer pane is a
# property of the session rather than of the daemon's answer, so the scenario
# that shows it says which session it was taken in.
session_type() {
  case "$1" in
    computer_wayland) echo wayland ;;
    *) echo x11 ;;
  esac
}

[ -f "$CRATE/Cargo.toml" ] || fail "no crate at $CRATE"

if [ "${1:-}" = "--container" ]; then
  CONTAINER=1
  shift
fi

if [ $# -gt 0 ]; then
  SCENARIOS=("$@")
fi

# The same script, run inside the image the crate is built in, under a display
# it makes for itself.
if [ "$CONTAINER" = "1" ]; then
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  docker image inspect "$IMAGE" >/dev/null 2>&1 ||
    fail "no $IMAGE image: run scripts/container_build.sh first"

  docker volume create "$CACHE_VOLUME" >/dev/null
  exec docker run --rm --init \
    -v "$ROOT_DIR:/workspace" \
    -v "$CACHE_VOLUME:/cache" \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e FERMIX_DESKTOP_TEST_ROOT=/tmp/fermix-desktop-tests \
    -w /workspace \
    "$IMAGE" xvfb-run -a scripts/capture.sh "${SCENARIOS[@]}"
fi

# The facts a reviewer needs to read a capture, taken from the machine that
# produced it rather than assumed.
COMMIT="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo uncommitted)"
GTK_VERSION="$(pkg-config --modversion gtk4)"
ADW_VERSION="$(pkg-config --modversion libadwaita-1)"
WINDOW_SIZE="880x560"
TEXT_SCALE="1.0"

mkdir -p "$OUT_DIR"
echo "capture: building"
cargo build --quiet --manifest-path "$CRATE/Cargo.toml" --bin fermix-desktop --bin fixture-daemon

TARGET_DIR="${CARGO_TARGET_DIR:-$CRATE/target}"
APP="$TARGET_DIR/debug/fermix-desktop"
DAEMON="$TARGET_DIR/debug/fixture-daemon"
[ -x "$APP" ] || fail "no application at $APP"
[ -x "$DAEMON" ] || fail "no fixture daemon at $DAEMON"

# Under /tmp rather than $TMPDIR: a Unix socket address is 104 bytes on macOS
# and 108 on Linux, and the scenario's home holds daemon.sock. macOS's per-user
# temporary directory is 49 bytes on its own, which leaves a scenario name no
# room and fails the connect rather than the bind.
WORK="$(mktemp -d /tmp/fermix-capture.XXXXXX)"
DAEMON_PID=""

cleanup() {
  if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

if [ ! -f "$INDEX" ]; then
  cat > "$INDEX" <<'HEADER'
# Reference captures

Rendered by the application's own capture mode against the fixture daemon, at
the default window size and text scale, in both colour schemes. Every row
carries what a reviewer needs to read the image: which fixtures produced it,
which commit drew it, and which toolkit drew it.

Take them with `scripts/capture.sh --container`. The ones that get reviewed are
the container's: it is Linux, it carries the toolkit floor, and it draws the
icons a desktop draws. A capture taken on a development Mac is honest about
layout and copy and silent about icons, because that host has no icon theme at
all.

A capture that exists is not an accepted capture. The reviewer column says who
last looked at the image: `automated review` is a review pass that opened it and
read it against `LINUX_DESIGN_SYSTEM_REDLINES.md`, and `pending` is nobody. The
decision column is the owner's, and no review pass fills it in. Taking a capture
again makes a new image, so both columns go back to `pending` with it.

| File | Fixture | Commit | GTK | libadwaita | Window | Text scale | Scheme | Reviewer | Decision |
|---|---|---|---|---|---|---|---|---|---|
HEADER
fi

for scenario in "${SCENARIOS[@]}"; do
  for scheme in "${SCHEMES[@]}"; do
    home="$WORK/$scenario"
    rm -rf "$home"
    mkdir -p "$home"

    FIXTURE_DAEMON_SCENARIO="$scenario" "$DAEMON" "$home" > "$home/daemon.out" 2>&1 &
    DAEMON_PID=$!

    # The daemon prints its socket path when it is ready to answer, and the
    # not-running scenario prints it and exits, which is the state itself.
    for _ in $(seq 1 50); do
      [ -s "$home/daemon.out" ] && break
      sleep 0.1
    done

    echo "capture: $scenario, $scheme"
    captured="$(
      FERMIX_DESKTOP_FIXTURE_HOME="$home" \
      FERMIX_DESKTOP_CLI="$CRATE/tests/fixtures/cli/fermix" \
      FERMIX_FAKE_CLI_STATE="$CRATE/tests/fixtures/cli/states/$(cli_state "$scenario")" \
      FERMIX_DESKTOP_STATE_DIR="$home/state" \
      FERMIX_DESKTOP_AUTOSTART_DIR="$home/autostart" \
      FIXTURE_DAEMON_SCENARIO="$scenario" \
      XDG_SESSION_TYPE="$(session_type "$scenario")" \
      ADW_DEBUG_COLOR_SCHEME="$scheme" \
      "$APP" --capture "$OUT_DIR"
    )" || fail "the application did not finish for $scenario"

    if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
      kill "$DAEMON_PID" 2>/dev/null || true
      wait "$DAEMON_PID" 2>/dev/null || true
    fi
    DAEMON_PID=""

    scheme_name="light"
    [ "$scheme" = "prefer-dark" ] && scheme_name="dark"

    while IFS= read -r file; do
      [ -n "$file" ] || continue
      name="$(basename "$file")"
      # One row per capture, replaced rather than repeated when it is taken
      # again.
      grep -v "^| \`$name\`" "$INDEX" > "$INDEX.next" || true
      mv "$INDEX.next" "$INDEX"
      printf '| `%s` | %s | %s | %s | %s | %s | %s | %s | pending | pending |\n' \
        "$name" "$scenario" "$COMMIT" "$GTK_VERSION" "$ADW_VERSION" \
        "$WINDOW_SIZE" "$TEXT_SCALE" "$scheme_name" >> "$INDEX"
    done <<< "$captured"
  done
done

echo "capture: $(find "$OUT_DIR" -name '*.png' | wc -l | tr -d ' ') images in $OUT_DIR"
