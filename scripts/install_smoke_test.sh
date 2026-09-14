#!/usr/bin/env bash
#
# Exercise install_smoke.sh's refusals and its container's declarations, without
# running the smoke.
#
# The gate itself takes minutes and needs a privileged container; its argument
# handling takes milliseconds, and a gate that accepted a package path that does
# not exist would report a pass having installed nothing. Docker is never invoked
# here.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/install_smoke.sh"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.smoke"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/install-smoke-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "install_smoke_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "install_smoke_test: the script parses"
bash -n "$SCRIPT" || fail "the script does not parse"
echo "  ok: shell syntax"

echo "install_smoke_test: refusals"

expect_refusal "no arguments at all" bash "$SCRIPT"
expect_refusal "an engine package with no desktop package" bash "$SCRIPT" "$WORK/engine.deb"

printf 'not a package\n' > "$WORK/desktop.deb"
expect_refusal "an engine package that does not exist" \
  bash "$SCRIPT" "$WORK/missing.deb" "$WORK/desktop.deb"
expect_refusal "a desktop package that does not exist" \
  bash "$SCRIPT" - "$WORK/missing.deb"

mkdir -p "$WORK/empty-bin"
expect_refusal "a host with no container runtime" \
  env PATH="$WORK/empty-bin" bash "$SCRIPT" - "$WORK/desktop.deb"

echo "install_smoke_test: the container it declares"
[ -f "$DOCKERFILE" ] || fail "no smoke container at $DOCKERFILE"

# Each of these is a gate's dependency rather than a convenience, and a missing
# one shows up as a smoke that looks broken rather than as a named refusal:
# systemd because a user unit with no user manager is a file nobody reads,
# polkitd because linger for another account goes through it, and the three X
# tools because the evidence this gate produces is a picture of a window and the
# class that window carries.
for needed in systemd systemd-sysv dbus dbus-user-session polkitd \
  xvfb xauth xdotool imagemagick; do
  grep -q "$needed" "$DOCKERFILE" || fail "the smoke container does not install $needed"
done
echo "  ok: every dependency the gate uses is installed"

grep -q 'useradd --create-home' "$DOCKERFILE" ||
  fail "the smoke container has no ordinary account to install the service as"
grep -q 'CMD \["/sbin/init"\]' "$DOCKERFILE" ||
  fail "the smoke container does not run systemd as process one"
grep -q 'STOPSIGNAL SIGRTMIN+3' "$DOCKERFILE" ||
  fail "the smoke container cannot be stopped cleanly, so a run leaks a container"
echo "  ok: systemd is process one, and one ordinary account owns the service"

echo "install_smoke_test: what the gate asserts"
# The facts the smoke exists to prove. A script that stopped asserting one
# of them would still exit zero, so the assertions themselves are checked here.
for assertion in \
  '"ok":true' \
  '"alignment":"aligned"' \
  '"enabled":true' \
  '"active":true' \
  'fermix-desktop --version' \
  'xdotool search --class'; do
  grep -qF -- "$assertion" "$SCRIPT" ||
    fail "the smoke no longer asserts $assertion"
done
echo "  ok: the install, the alignment, the unit's own state, the version and the window class"

echo "install_smoke_test: every refusal fired"
