#!/usr/bin/env bash
#
# The tray's D-Bus path, against a real session bus, in the build container.
#
# Every other tray test is a pure-value test. This is the one that puts bytes on
# a bus and makes a stub host do what the GNOME AppIndicator extension does:
# take the registration, read the item's properties, follow `Menu` to the
# dbusmenu object, ask for the layout, and click each row.
#
# WHY THE ARMS ARE MARKED #[ignore]. They need a private bus each, so an
# ordinary `cargo test` cannot run them and would go red for a reason that has
# nothing to do with the code. Ignored there, run here -- and this script
# refuses an arm that reports no test run, because "ignored" and "passed" must
# never be allowed to look alike.
#
# WHY EACH TEST GETS ITS OWN BUS. The watcher name is fixed by the
# specification, so the arm that asserts nothing owns it cannot share a bus with
# the arms that own it, and cargo runs a test file's tests in parallel threads
# of one process with one bus. So they are run one at a time, each under its own
# `dbus-run-session`. That is slower and it is the only arrangement in which
# each arm measures what it claims to.
#
# WHY A PASSWD ENTRY IS MOUNTED. The container runs as the invoking uid so that
# nothing it writes into the tree is root-owned, and that uid has no entry in
# the image's /etc/passwd. dbus-daemon looks the caller up before it will start
# and fails with "Could not get password database information for UID" -- which
# reads like a memory error and is not one. A generated passwd and group file
# are mounted so the lookup succeeds.
#
# THIS IS NOT THE OWNER'S DESKTOP. Nothing here touches the owner's session: the
# bus is private to the container and dies with it, no window is mapped on any
# real display, and no icon appears on any real panel. The owner's panel remains
# the final check, in the acceptance runbook, as a row a person can falsify.
#
# Usage:
#   scripts/tray_smoke.sh            run every arm, each on its own bus
#   scripts/tray_smoke.sh <name>     run one arm by test name
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"
CACHE_VOLUME="${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}"
PREFIX="/usr/lib/fermix-desktop"

export DOCKER_HOST="${DOCKER_HOST:-unix:///var/run/docker.sock}"

fail() {
  echo "tray_smoke: $*" >&2
  exit 1
}

command -v docker >/dev/null 2>&1 || fail "docker is not installed"
docker image inspect "$IMAGE" >/dev/null 2>&1 ||
  fail "no build image at $IMAGE; run scripts/container_build.sh first"

# The arms, named so that one can be run alone and so that this list is the one
# place that says what the gate covers.
ARMS=(
  a_host_takes_the_item_and_can_read_its_whole_menu
  every_clickable_row_reaches_the_action_it_names
  a_hover_does_not_fire_the_row
  with_no_watcher_on_the_bus_the_item_says_so_rather_than_failing
  the_stub_host_would_reject_a_layout_it_could_not_parse
)

if [ "$#" -gt 0 ]; then
  ARMS=("$1")
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/tray-smoke.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT

# A passwd and group the container's uid appears in. See the header.
printf 'root:x:0:0:root:/root:/bin/sh\nbuilder:x:%s:%s:builder:/tmp:/bin/sh\n' \
  "$(id -u)" "$(id -g)" > "$WORK/passwd"
printf 'root:x:0:\nbuilder:x:%s:\n' "$(id -g)" > "$WORK/group"

run_arm() {
  local arm="$1"

  docker run --rm --init \
    --user "$(id -u):$(id -g)" \
    -v "$ROOT_DIR:/workspace" \
    -v "$CACHE_VOLUME:/cache" \
    -v "$WORK/passwd:/etc/passwd:ro" \
    -v "$WORK/group:/etc/group:ro" \
    -e HOME=/tmp \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e "LD_LIBRARY_PATH=$PREFIX/lib" \
    -w /workspace/App/Fermix \
    "$IMAGE" \
    dbus-run-session -- cargo test --test tray_bus -- --ignored --exact --nocapture "$arm"
}

failed=0
for arm in "${ARMS[@]}"; do
  echo "tray_smoke: == $arm"
  if run_arm "$arm" > "$WORK/$arm.log" 2>&1; then
    # A pass that ran nothing is not a pass. cargo prints "0 passed" for a
    # filter that matched no test, and an exact filter with a typo in it does
    # precisely that while exiting zero.
    if grep -q "1 passed" "$WORK/$arm.log"; then
      echo "  ok"
    else
      echo "  REFUSED: the filter matched no test, so nothing was measured" >&2
      grep -E "test result" "$WORK/$arm.log" >&2 || true
      failed=1
    fi
  else
    echo "  FAILED" >&2
    grep -E "panicked|assertion|test result|^error" "$WORK/$arm.log" >&2 | head -12
    failed=1
  fi
done

[ "$failed" = "0" ] || fail "at least one arm did not hold"
echo "tray_smoke: every arm holds, each on its own session bus"
