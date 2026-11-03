#!/usr/bin/env bash
# Offline tests for check_contracts.py against a stub gh serving a fixture engine: contracts that are
# the pinned engine's files byte for byte pass, and a contract re-vendored from another engine, a
# file changed by hand, a SOURCE.md naming another commit, and every answer the check cannot read
# are refused. Nothing here reaches the network.
#   desktop/scripts/check_contracts_test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
desktop=$(dirname "$here")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
PINNED=0123456789abcdef0123456789abcdef01234567
OTHER=89abcdef0123456789abcdef0123456789abcdef
FILES=(PROTOCOL.md protocol.schema.json fixtures/events.jsonl)

fail() {
  echo "check_contracts_test: $*" >&2
  exit 1
}

# A gh that serves `gh api ... repos/<repo>/contents/<path>?ref=<commit>` from
# $STUB_ENGINE/<commit>/<path>, fails each call while $STUB_FAILURES says so, and logs every call.
mkdir -p "$work/bin"
cat > "$work/bin/gh" << 'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$STUB_LOG"
if [ -s "$STUB_FAILURES" ]; then
  sed -i '1d' "$STUB_FAILURES"
  echo "gh: Server Error (HTTP 502)" >&2
  exit 1
fi
request="${*: -1}"
path="${request#repos/tezra-io/fermix/contents/}"
[ "$path" != "$request" ] || { echo "stub gh: not a contents request: $request" >&2; exit 2; }
file="$STUB_ENGINE/${path#*\?ref=}/${path%%\?ref=*}"
[ -f "$file" ] || { echo "gh: Not Found (HTTP 404)" >&2; exit 1; }
cat "$file"
STUB
chmod +x "$work/bin/gh"
export PATH="$work/bin:$PATH" STUB_LOG="$work/gh.log" STUB_FAILURES="$work/failures"
export STUB_ENGINE="$work/engine"
: > "$STUB_FAILURES"

# The real pin, naming the fixture commit.
python3 -I - "$desktop/engine/PIN.json" "$work/pin.json" "$PINNED" << 'PY'
import json, sys
pin = json.load(open(sys.argv[1]))
pin["source_commit"] = sys.argv[3]
json.dump(pin, open(sys.argv[2], "w"), indent=2)
PY

# One contract as the engine serves it at the pinned commit, and its vendored copy: the same three
# files, a CHECKSUMS.txt listing them, and a SOURCE.md naming <commit> and each file's engine path.
contract() {
  local name="$1" commit="$2" dir="$work/contracts/$1"
  local served="$STUB_ENGINE/$PINNED/apps/fermix_core/priv/$1"
  mkdir -p "$served/fixtures" "$dir"
  printf '# The %s protocol\n\nVersion 2, on 1..2.\n' "$name" > "$served/PROTOCOL.md"
  printf '{"x-protocol-version": 2, "x-supported-version-range": {"min": 1, "max": 2}}\n' \
    > "$served/protocol.schema.json"
  printf '{"type": "hello", "name": "%s"}\n' "$name" > "$served/fixtures/events.jsonl"
  cp -r "$served/." "$dir/"
  (cd "$dir" && sha256sum "${FILES[@]}" > CHECKSUMS.txt)
  {
    printf '# Where this contract comes from\n\n| file here | engine path |\n|---|---|\n'
    for file in "${FILES[@]}"; do
      printf '| `%s` | `apps/fermix_core/priv/%s/%s` |\n' "$file" "$name" "$file"
    done
    printf '\n- Repository: fermix (the engine)\n- Commit: `%s`, the engine v0.0.1.\n' "$commit"
  } > "$dir/SOURCE.md"
}

reset() {
  rm -rf "$work/contracts" "$STUB_ENGINE"
  : > "$STUB_LOG"
  : > "$STUB_FAILURES"
}

check() {
  "$here/check_contracts.py" --pin "$work/pin.json" --contracts "$work/contracts"
}

expect_pass() {
  local what="$1"
  check > "$work/out" 2>&1 || fail "$what was refused: $(cat "$work/out")"
  echo "  passed: $what"
}

# Runs the check, which must fail, and checks that what it printed gives the expected reason.
expect_refusal() {
  local what="$1" reason="$2"
  if check > "$work/out" 2>&1; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$work/out" || fail "$what was refused for another reason: $(cat "$work/out")"
  echo "  refused: $what"
}

echo "check_contracts_test: contracts that are the pinned engine's, byte for byte"
reset
contract management "$PINNED"
contract companion "$PINNED"
contract realtime "$PINNED"
expect_pass "three contracts at the pinned commit"
[ "$(grep -c "ref=$PINNED\$" "$STUB_LOG")" -eq 9 ] ||
  fail "one read of each file at the pinned commit was expected: $(cat "$STUB_LOG")"

echo "check_contracts_test: a contract that is not the pinned engine's"
reset
contract management "$PINNED"
contract companion "$PINNED"
printf '{"x-protocol-version": 4, "x-supported-version-range": {"min": 3, "max": 4}}\n' \
  > "$work/contracts/companion/protocol.schema.json"
(cd "$work/contracts/companion" && sha256sum "${FILES[@]}" > CHECKSUMS.txt)
expect_refusal "companion 4 vendored beside an engine serving 1 to 2" \
  "companion: protocol.schema.json is not the pinned engine's"
reset
contract realtime "$PINNED"
printf '{"type": "hello", "name": "edited"}\n' > "$work/contracts/realtime/fixtures/events.jsonl"
expect_refusal "a fixture changed by hand" "realtime: fixtures/events.jsonl is not the pinned engine's"

echo "check_contracts_test: a contract that names another commit"
reset
contract management "$PINNED"
contract realtime "$OTHER"
expect_refusal "realtime from another commit" \
  "realtime: SOURCE.md names $OTHER, and the pin names $PINNED"

echo "check_contracts_test: what the check cannot read"
reset
contract management "$PINNED"
contract companion "$PINNED"
rm "$work/contracts/companion/SOURCE.md"
expect_refusal "a contract without SOURCE.md" "companion: SOURCE.md cannot be read"
reset
contract companion "$PINNED"
printf '| `fixtures/extra.jsonl` | `apps/fermix_core/priv/companion/fixtures/extra.jsonl` |\n' \
  >> "$work/contracts/companion/SOURCE.md"
expect_refusal "a SOURCE.md naming a file CHECKSUMS.txt does not list" \
  "companion: SOURCE.md names fixtures/extra.jsonl, which CHECKSUMS.txt does not list"
reset
contract companion "$PINNED"
sed -i '/^- Commit:/d' "$work/contracts/companion/SOURCE.md"
expect_refusal "a SOURCE.md naming no commit" "companion: SOURCE.md names no commit"
reset
contract companion "$PINNED"
sed -i '/`PROTOCOL.md`/d' "$work/contracts/companion/SOURCE.md"
expect_refusal "a SOURCE.md naming no engine path for a listed file" \
  "companion: SOURCE.md names no engine path for PROTOCOL.md"
reset
contract companion "$PINNED"
rm "$work/contracts/companion/CHECKSUMS.txt"
expect_refusal "a contract without CHECKSUMS.txt" \
  "companion: CHECKSUMS.txt is missing, empty or not sha256sum's format"
reset
contract companion "$PINNED"
: > "$work/contracts/companion/CHECKSUMS.txt"
expect_refusal "an empty CHECKSUMS.txt" \
  "companion: CHECKSUMS.txt is missing, empty or not sha256sum's format"
reset
contract companion "$PINNED"
rm "$work/contracts/companion/fixtures/events.jsonl"
expect_refusal "a listed file missing here" \
  "companion: fixtures/events.jsonl is listed in CHECKSUMS.txt and cannot be read here"
reset
contract companion "$PINNED"
rm "$STUB_ENGINE/$PINNED/apps/fermix_core/priv/companion/protocol.schema.json"
expect_refusal "an engine without the file, naming the repository and the token" \
  "companion: the pinned engine's protocol.schema.json could not be read from tezra-io/fermix at $PINNED; the token gh uses (GH_TOKEN) must be able to read tezra-io/fermix"
reset
mkdir -p "$work/contracts"
expect_refusal "no contracts at all" "no vendored contracts under"

echo "check_contracts_test: gh is retried a bounded number of times"
reset
contract companion "$PINNED"
printf 'fail\nfail\n' > "$STUB_FAILURES"
expect_pass "two failed reads, then an answer"
[ "$(wc -l < "$STUB_LOG")" -eq 5 ] || fail "five reads were expected: $(cat "$STUB_LOG")"
reset
contract companion "$PINNED"
printf 'fail\nfail\nfail\n' > "$STUB_FAILURES"
expect_refusal "a read that fails every attempt" \
  "companion: the pinned engine's PROTOCOL.md could not be read"
[ "$(grep -c '/PROTOCOL.md?' "$STUB_LOG")" -eq 3 ] ||
  fail "PROTOCOL.md was not read exactly 3 times: $(cat "$STUB_LOG")"

echo "check_contracts_test: the pin itself is checked"
reset
contract companion "$PINNED"
echo '{}' > "$work/broken-pin.json"
if "$here/check_contracts.py" --pin "$work/broken-pin.json" --contracts "$work/contracts" \
  > "$work/out" 2>&1; then
  fail "a broken pin was accepted"
fi
grep -qF "engine pin" "$work/out" || fail "a broken pin was refused for another reason: $(cat "$work/out")"
echo "  refused: a pin engine_pin.py refuses"

# The stub serves this tree's own files at the pin, so this holds what can be held offline: every
# SOURCE.md names the pin and an engine path for each file it lists. Byte identity with the real
# engine is the release's live check.
echo "check_contracts_test: this tree's contracts name this tree's pin"
pin_commit=$("$here/engine_pin.py" --get source_commit)
reset
for dir in "$desktop"/core/contracts/*/; do
  name=$(basename "$dir")
  while read -r _ file; do
    path=$(sed -n "s#^| \`$file\` | \`\(.*\)\` |\$#\1#p" "$dir/SOURCE.md")
    [ -n "$path" ] || fail "$name's SOURCE.md names no engine path for $file"
    mkdir -p "$STUB_ENGINE/$pin_commit/$(dirname "$path")"
    cp "$dir/$file" "$STUB_ENGINE/$pin_commit/$path"
  done < "$dir/CHECKSUMS.txt"
done
"$here/check_contracts.py" --contracts "$desktop/core/contracts" > "$work/out" 2>&1 ||
  fail "this tree's contracts do not name its pin: $(cat "$work/out")"
echo "  passed: this tree's contracts"

echo "check_contracts_test: ok"
