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
#
# A row is replaced per capture taken, and never pruned. Retiring a scenario
# therefore leaves its rows in INDEX.md describing images that no longer exist,
# and a full re-take cannot notice: it rewrites every row that still exists and
# has no way to see one that does not. A stale row is worse than a stale image,
# because an image can be opened and found wrong, while a row whose file is gone
# cannot be opened at all — and it carries a reviewer and a decision column, so
# it reads as settled. Delete the rows by hand when you delete a capture.
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
           onboarding_restart_needed onboarding_ready onboarding_skew \
           secret_store_file)
SCHEMES=(prefer-light prefer-dark)

IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"
CACHE_VOLUME="${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}"
# The private toolkit's installed prefix, the same constant the crate compiles
# in as `runtime::PRIVATE_PREFIX` and `scripts/container_build.sh` spells as
# PREFIX. A capture binary lives in target/debug, where $ORIGIN/../lib does not
# resolve, which is the one documented case for LD_LIBRARY_PATH.
PREFIX="/usr/lib/fermix-desktop"
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
#
# Three things here are the same three the crate's test run needs, and for the
# same reasons, so the display argument comes from `scripts/crate_gates.sh`
# rather than being spelled a third time. The gate path and the release path
# already drifted apart once over exactly this.
if [ "$CONTAINER" = "1" ]; then
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  docker image inspect "$IMAGE" >/dev/null 2>&1 ||
    fail "no $IMAGE image: run scripts/container_build.sh first"

  # shellcheck source=scripts/crate_gates.sh
  . "$ROOT_DIR/scripts/crate_gates.sh"

  docker volume create "$CACHE_VOLUME" >/dev/null
  # The cache belongs to the invoking account, or the capture run writes PNGs
  # into the tree as root and the next person cannot replace them.
  docker run --rm \
    -v "$CACHE_VOLUME:/cache" \
    --entrypoint chown alpine:latest -R "$(id -u):$(id -g)" /cache >/dev/null 2>&1 ||
    true

  exec docker run --rm --init \
    -u "$(id -u):$(id -g)" \
    -v "$ROOT_DIR:/workspace" \
    -v "$CACHE_VOLUME:/cache" \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e FERMIX_DESKTOP_TEST_ROOT=/tmp/fermix-desktop-tests \
    -e "LD_LIBRARY_PATH=$PREFIX/lib" \
    -e HOME=/tmp \
    -w /workspace \
    "$IMAGE" xvfb-run "${XVFB_ARGUMENTS[@]}" scripts/capture.sh "${SCENARIOS[@]}"
fi

# The facts a reviewer needs to read a capture, taken from the machine that
# produced it rather than assumed.
COMMIT="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo uncommitted)"
GTK_VERSION="$(pkg-config --modversion gtk4)"
ADW_VERSION="$(pkg-config --modversion libadwaita-1)"
WINDOW_SIZE="880x560"
TEXT_SCALE="1.0"

# Which private toolkit drew these. The key is the runtime lock file's, which is
# what names the published runtime image, and it belongs in the index because a
# capture is a picture of a toolkit as much as of a layout.
RUNTIME_KEY="$("$ROOT_DIR/packaging/runtime/build_runtime.sh" --print-key 2>/dev/null || echo unknown)"

# And a guard, because the key is computed from the lock file while the pictures
# are drawn by whatever the image happens to carry. A build image that was not
# rebuilt after a lock change would put a key in the index that describes a
# toolkit nothing here ever ran, which is worse than recording no key at all:
# the index is read later by someone deciding whether a capture is current.
lock_version() {
  python3 - "$1" <<'PYTHON'
import json
import sys

name = sys.argv[1]
with open("packaging/runtime/RUNTIME.lock.json", encoding="utf-8") as handle:
    locked = json.load(handle)
for component in locked["components"]:
    if component["name"] == name:
        print(component["version"])
        break
PYTHON
}

LOCKED_GTK="$(cd "$ROOT_DIR" && lock_version gtk)"
LOCKED_ADW="$(cd "$ROOT_DIR" && lock_version libadwaita)"
if [ "$LOCKED_GTK" != "$GTK_VERSION" ] || [ "$LOCKED_ADW" != "$ADW_VERSION" ]; then
  fail "the lock file names gtk $LOCKED_GTK and libadwaita $LOCKED_ADW, and this image
  carries gtk $GTK_VERSION and libadwaita $ADW_VERSION. The image predates the lock
  file, so runtime key $RUNTIME_KEY would describe a toolkit that drew none of these
  captures. Rebuild the build image against the current runtime tree first."
fi

# The version comparison above is necessary and not sufficient, and the gap is
# not hypothetical: runtimes ac37ab970c0e971c and a3f02e1ab136fbc5 both report
# gtk 4.16.7 and libadwaita 1.6.9 while hashing to different trees, so a build
# image carrying the first passes every check above while the lock file names
# the second. That is how a whole capture batch can be drawn by a toolkit the
# index then misreports. The only cheap way to settle identity from inside the
# image is for the runtime to have written its own key into the prefix, which
# it now does: the runtime build stamps identity.json with the cache key that
# names it. Read that rather than any property the key is derived from.
RUNTIME_KEY_FILE="$PREFIX/share/fermix-desktop-runtime/identity.json"
if [ -r "$RUNTIME_KEY_FILE" ]; then
  INSTALLED_KEY="$(python3 -c '
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    print(json.load(handle)["cache_key"])
' "$RUNTIME_KEY_FILE")"
  if [ "$INSTALLED_KEY" != "$RUNTIME_KEY" ]; then
    fail "the lock file names runtime key $RUNTIME_KEY and the toolkit installed in
  $PREFIX says it is $INSTALLED_KEY. These captures would be drawn by one toolkit
  and filed under another. Rebuild the build image against the current runtime."
  fi
elif [ "${CAPTURE_ALLOW_UNVERIFIED_RUNTIME:-}" = "1" ]; then
  # Recorded rather than waved through: an index row that admits the key was
  # never checked is honest, and a reviewer can act on it. A row that silently
  # claims a key nobody verified is the failure this guard exists to prevent.
  echo "capture: warning, $RUNTIME_KEY_FILE is absent, so the toolkit drawing these" >&2
  echo "capture: captures is unverified and the index will say so" >&2
  RUNTIME_KEY="unverified:$RUNTIME_KEY"
else
  fail "$RUNTIME_KEY_FILE does not exist, so nothing in this image says which runtime
  it carries, and the versions above cannot tell two runtimes apart. The runtime has
  stamped this file since key a89ab0607a323501, so an image without it predates the
  freeze and should be rebuilt. To take the batch anyway, set
  CAPTURE_ALLOW_UNVERIFIED_RUNTIME=1 and every index row will record the key as
  unverified."
fi

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
      printf '| `%s` | %s | %s | %s | %s | %s | %s | %s | %s | pending | pending |\n' \
        "$name" "$scenario" "$COMMIT" "$GTK_VERSION" "$ADW_VERSION" "$RUNTIME_KEY" \
        "$WINDOW_SIZE" "$TEXT_SCALE" "$scheme_name" >> "$INDEX"
    done <<< "$captured"
  done
done

echo "capture: $(find "$OUT_DIR" -name '*.png' | wc -l | tr -d ' ') images in $OUT_DIR"
