#!/usr/bin/env bash
# Offline tests for fetch_engine.sh against a stub gh serving a fixture release: it downloads exactly
# the pinned packages and their sidecars, retries a failed download at most 3 times, and refuses
# before any download when the pin or the directory is wrong. Nothing here reaches the network.
#   desktop/scripts/fetch_engine_test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/fixtures/engine"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export PATH="$fixtures/bin:$PATH"

COMMIT=0123456789abcdef0123456789abcdef01234567
AMD64=(fermix_0.0.1_amd64.deb fermix-0.0.1-1.x86_64.rpm)
ARM64=(fermix_0.0.1_arm64.deb fermix-0.0.1-1.aarch64.rpm)

fail() {
  echo "fetch_engine_test: $*" >&2
  exit 1
}

# Runs a command that must fail, and checks that its stderr gives the expected reason.
expect_refusal() {
  local what="$1" reason="$2" err="$work/stderr"
  shift 2
  if "$@" > /dev/null 2> "$err"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$err" || fail "$what was refused for another reason: $(cat "$err")"
  echo "  refused: $what"
}

# The names a fetch of these packages must leave behind: each package and its three sidecars.
expected_files() {
  local package suffix
  for package in "$@"; do
    for suffix in "" .sha256 .sig .pem; do
      printf '%s\n' "$package$suffix"
    done
  done | sort
}

# The directory holds exactly the expected files, each the bytes the release published.
expect_fetched() {
  local dir="$1" name
  shift
  diff <(expected_files "$@") <(find "$dir" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort) > "$work/diff" ||
    fail "$dir does not hold exactly the pinned files: $(cat "$work/diff")"
  while read -r name; do
    cmp -s "$work/release/assets/$name" "$dir/$name" || fail "$name is not the published file"
  done < <(expected_files "$@")
}

"$fixtures/fixture_release.sh" "$work/release" "$fixtures/fermix_0.0.1_amd64.deb" \
  "$fixtures/fermix-0.0.1-1.x86_64.rpm" "$COMMIT" v0.0.1
export FIXTURE_GH_RELEASE="$work/release"
"$here/engine_pin.py" --pin "$work/pin.json" v0.0.1 > /dev/null

echo "fetch_engine_test: every pinned package"
"$here/fetch_engine.sh" --pin "$work/pin.json" "$work/all" > /dev/null
expect_fetched "$work/all" "${AMD64[@]}" "${ARM64[@]}"
echo "  ok: 4 packages and their 12 sidecars, nothing else"

echo "fetch_engine_test: one architecture"
"$here/fetch_engine.sh" --pin "$work/pin.json" "$work/amd64" amd64 > /dev/null
expect_fetched "$work/amd64" "${AMD64[@]}"
"$here/fetch_engine.sh" --pin "$work/pin.json" "$work/aarch64" aarch64 > /dev/null
expect_fetched "$work/aarch64" "${ARM64[@]}"
echo "  ok: amd64, and arm64 by its rpm name"

echo "fetch_engine_test: a download that fails twice and then arrives"
FIXTURE_GH_LOG="$work/flaky.log" FIXTURE_GH_FAIL_DOWNLOADS=2 FIXTURE_SLEEP_LOG="$work/flaky.sleep" \
  "$here/fetch_engine.sh" --pin "$work/pin.json" "$work/flaky" amd64 > /dev/null 2>&1
expect_fetched "$work/flaky" "${AMD64[@]}"
[ "$(grep -c 'fermix_0.0.1_amd64.deb$' "$work/flaky.log")" -eq 3 ] ||
  fail "the first package was not asked for three times: $(cat "$work/flaky.log")"
[ "$(wc -l < "$work/flaky.sleep")" -eq 2 ] || fail "two failures did not pause twice"
echo "  ok: two retries, then the file, with no partial file left over"

echo "fetch_engine_test: a download that never arrives"
mkdir "$work/broken"
expect_refusal "a download that fails four times" "fermix_0.0.1_amd64.deb did not download in 4 attempts" \
  env FIXTURE_GH_LOG="$work/broken.log" FIXTURE_GH_FAIL_DOWNLOADS=1000 \
  "$here/fetch_engine.sh" --pin "$work/pin.json" "$work/broken" amd64
[ "$(grep -c '^download ' "$work/broken.log")" -eq 4 ] ||
  fail "a broken download was not tried exactly once and retried 3 times: $(cat "$work/broken.log")"
[ -z "$(ls -A "$work/broken")" ] || fail "a failed fetch left $(ls -A "$work/broken") behind"
echo "  ok: one attempt and 3 retries, then a refusal, and nothing left behind"

echo "fetch_engine_test: refusals before any download"
: > "$work/quiet.log"
mkdir "$work/occupied"
touch "$work/occupied/left-over"
expect_refusal "a directory that already holds files" "already holds files" \
  env FIXTURE_GH_LOG="$work/quiet.log" "$here/fetch_engine.sh" --pin "$work/pin.json" "$work/occupied"
python3 - "$work/pin.json" "$work/half.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    pin = json.load(handle)
pin["packages"]["amd64"]["deb"]["sha256"] = None
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    json.dump(pin, handle)
PY
expect_refusal "a half-filled pin" "half filled" \
  env FIXTURE_GH_LOG="$work/quiet.log" "$here/fetch_engine.sh" --pin "$work/half.json" "$work/half"
expect_refusal "an architecture nobody builds" "riscv64" \
  env FIXTURE_GH_LOG="$work/quiet.log" "$here/fetch_engine.sh" --pin "$work/pin.json" "$work/riscv" riscv64
expect_refusal "no output directory" "usage" "$here/fetch_engine.sh" --pin "$work/pin.json"
[ ! -s "$work/quiet.log" ] || fail "a refused fetch still called gh: $(cat "$work/quiet.log")"
echo "  ok: gh was never called"

echo "fetch_engine_test: ok"
