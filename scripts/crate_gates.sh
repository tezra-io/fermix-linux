#!/usr/bin/env bash
# What the crate's test run needs, in the one place both scripts read it.
#
# scripts/container_build.sh and scripts/build_packages.sh both run the crate's
# tests inside the build container, and for a while they disagreed about how:
# one had a display argument the other did not, and a test failed in the release
# path that passed in the gate path. Two spellings of one rule is a bug waiting
# for the day they differ, so the rule lives here and neither script states it.
#
# Sourced, not run.

# Xvfb's default screen is 640x480. GTK clamps a window to the screen, so the
# window the widget test opens at 900x620 comes back as 640x480 and the test
# reads it as a window-state feature that does not work. It is a display that is
# too small. Anything comfortably larger than the largest window the tests open
# will do; this is a plain desktop size.
# shellcheck disable=SC2034  # read by the scripts that source this file
XVFB_ARGUMENTS=(-a -s "-screen 0 1280x1024x24")

# How many test binaries this crate has, asked of cargo rather than counted by
# hand: `cargo test --no-run` builds every test target and says where each
# executable landed, so the answer moves when the crate does and a hard-coded
# number cannot go stale.
crate_test_binary_count() {
  local crate_dir="$1"
  (
    cd "$crate_dir" || return 1
    cargo test --no-run --message-format=json 2>/dev/null |
      python3 -c '
import json
import sys

count = 0
for line in sys.stdin:
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        message = json.loads(line)
    except ValueError:
        continue
    # One line per built artifact; a test target is the one with an executable
    # and the test profile set.
    if message.get("reason") != "compiler-artifact":
        continue
    if not message.get("executable"):
        continue
    if not message.get("profile", {}).get("test"):
        continue
    count += 1
print(count)
'
  )
}

# The guard that matters more than any single test: prove the suite RAN.
#
# `cargo test --no-fail-fast` carries on past a test binary that cannot start,
# and a binary that dies at load with exit 127 prints no "test result" line at
# all. A reader skimming for failures sees nothing but "ok" and a run that
# skipped a hundred tests looks exactly like a run that passed them. So the
# number of results is compared with the number of test binaries cargo built.
check_every_test_binary_reported() {
  local output="$1" expected="$2"
  local reported doctests
  reported="$(grep -c '^test result:' "$output" || true)"

  # The documentation tests print a result line of their own and are not one of
  # the binaries cargo built, so they are subtracted rather than left to make
  # the two numbers disagree by one for a reason that is not a problem.
  doctests="$(grep -c 'Doc-tests' "$output" || true)"
  reported=$((reported - doctests))

  if [ -z "$expected" ] || [ "$expected" -lt 1 ] 2>/dev/null; then
    echo "crate_gates: cargo did not say how many test binaries it built" >&2
    return 1
  fi
  if [ "$reported" -ne "$expected" ]; then
    echo "crate_gates: cargo built $expected test binaries and $reported reported a result;" >&2
    echo "  a binary that dies at load prints nothing and --no-fail-fast carries on," >&2
    echo "  so this run did not test what it appears to have tested" >&2
    grep -E 'error while loading shared libraries|exit status: 127' "$output" >&2 || true
    return 1
  fi
  echo "  $reported test binaries built, $reported reported a result"
}
