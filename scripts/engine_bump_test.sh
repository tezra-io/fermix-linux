#!/usr/bin/env bash
#
# Exercise engine_bump.py's refusals against a throwaway copy of the four files
# it writes.
#
# The payload this script reads arrives from another repository over the
# network, so its validation is the boundary between "the engine repository said
# so" and "this repository wrote it down". Every refusal below is a shape a
# dispatch could actually carry: a tag that is not a release tag, a certificate
# identity pointing at somebody else's workflow, an asset name the engine
# release does not publish, a digest that is not a digest, a field nobody reads.
#
# Nothing here touches the network, the checked-in tree or Docker.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/engine_bump.py"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/engine-bump-test.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT

TAG="v9.9.9"
VERSION="9.9.9"
COMMIT="1111111111111111111111111111111111111111"
DIGEST_X86="2222222222222222222222222222222222222222222222222222222222222222"
DIGEST_ARM="3333333333333333333333333333333333333333333333333333333333333333"
IDENTITY="https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/$TAG"

fail() {
  echo "engine_bump_test: $*" >&2
  exit 1
}

# A tree with the four files the script edits, and nothing else. Built rather
# than copied so that a refusal here is never a refusal about the real tree.
make_tree() {
  local tree="$1"
  mkdir -p "$tree/engine" "$tree/App/Fermix" "$tree/packaging"
  cat > "$tree/engine/PIN.json" <<'JSON'
{
  "schema_version": 2,
  "repository": "tezra-io/fermix",
  "certificate_oidc_issuer": "https://token.actions.githubusercontent.com",
  "tag": null,
  "engine_version": null,
  "source_commit": null,
  "certificate_identity": null,
  "artifacts": {
    "linux_x86_64": { "asset": null, "sha256": null },
    "linux_aarch64": { "asset": null, "sha256": null }
  },
  "note": "unpinned, and this is how to fill it in"
}
JSON
  printf '[package]\nname = "fermix-desktop"\nversion = "0.0.1"\nedition = "2021"\n' \
    > "$tree/App/Fermix/Cargo.toml"
  printf '[[package]]\nname = "fermix-desktop"\nversion = "0.0.1"\ndependencies = []\n' \
    > "$tree/App/Fermix/Cargo.lock"
  printf '<component>\n  <releases>\n    <release version="0.0.1" date="2020-01-01" />\n  </releases>\n</component>\n' \
    > "$tree/packaging/io.tezra.Fermix.metainfo.xml"
}

# The payload the engine's dispatch job actually sends, with one field replaced
# by whatever the case under test wants to try.
write_payload() {
  local path="$1"
  shift
  PAYLOAD_TAG="$TAG" PAYLOAD_VERSION="$VERSION" PAYLOAD_COMMIT="$COMMIT" \
  PAYLOAD_IDENTITY="$IDENTITY" PAYLOAD_X86="$DIGEST_X86" PAYLOAD_ARM="$DIGEST_ARM" \
  PAYLOAD_EDITS="$*" python3 - "$path" <<'PY'
import json
import os
import sys

payload = {
    "tag": os.environ["PAYLOAD_TAG"],
    "version": os.environ["PAYLOAD_VERSION"],
    "source_commit": os.environ["PAYLOAD_COMMIT"],
    "certificate_identity": os.environ["PAYLOAD_IDENTITY"],
    "artifacts": {
        "linux_x86_64": {
            "asset": "fermix_app_engine_linux_x86_64.tar.gz",
            "sha256": os.environ["PAYLOAD_X86"],
        },
        "linux_aarch64": {
            "asset": "fermix_app_engine_linux_aarch64.tar.gz",
            "sha256": os.environ["PAYLOAD_ARM"],
        },
    },
}

# Each edit is `dotted.path=<json>`, or `-dotted.path` to delete the key.
for edit in os.environ["PAYLOAD_EDITS"].split():
    if edit.startswith("-"):
        keys = edit[1:].split(".")
        target = payload
        for key in keys[:-1]:
            target = target[key]
        del target[keys[-1]]
        continue
    path, _, raw = edit.partition("=")
    keys = path.split(".")
    target = payload
    for key in keys[:-1]:
        target = target[key]
    target[keys[-1]] = json.loads(raw)

with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump(payload, handle)
PY
}

expect_refusal() {
  local what="$1"
  shift
  local tree="$WORK/refuse"
  rm -rf "$tree"
  make_tree "$tree"
  write_payload "$WORK/payload.json" "$@"
  if python3 "$SCRIPT" --payload "$WORK/payload.json" --root "$tree" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  # A refused payload leaves the tree exactly as it was: the pin is the file a
  # later reader trusts, and a half-written bump is the state this script exists
  # to make impossible.
  grep -q '"tag": null' "$tree/engine/PIN.json" ||
    fail "$what was refused and the pin was written anyway"
  echo "  refused: $what"
}

echo "engine_bump_test: the script parses"
python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" "$SCRIPT" ||
  fail "the script does not parse"
echo "  ok: python syntax"

echo "engine_bump_test: refusals"
expect_refusal "a tag that is not an engine release tag" 'tag="v9.9"'
expect_refusal "a tag carrying a prerelease suffix" 'tag="v9.9.9-rc1"'
expect_refusal "a version that is not the tag's" 'version="9.9.8"'
expect_refusal "a source commit that is not 40 hex" 'source_commit="abc"'
expect_refusal "a certificate identity naming another repository" \
  'certificate_identity="https://github.com/evil/fermix/.github/workflows/release.yml@refs/tags/v9.9.9"'
expect_refusal "a certificate identity naming another workflow" \
  'certificate_identity="https://github.com/tezra-io/fermix/.github/workflows/evil.yml@refs/tags/v9.9.9"'
expect_refusal "a certificate identity bound to another tag" \
  'certificate_identity="https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/v1.0.0"'
expect_refusal "an asset name the engine release does not publish" \
  'artifacts.linux_x86_64.asset="fermix_app_engine_linux_x86_64.tar.gz.evil"'
expect_refusal "a digest that is not a digest" 'artifacts.linux_x86_64.sha256="0"'
expect_refusal "an artifact entry carrying a field nobody reads" \
  'artifacts.linux_x86_64.url="https://example.invalid"'
expect_refusal "a payload carrying a field nobody reads" 'extra="whatever"'
expect_refusal "a payload missing the source commit" '-source_commit'
expect_refusal "a payload naming a third target" \
  'artifacts.linux_riscv64={"asset":"x","sha256":"y"}'
expect_refusal "a payload naming only one target" '-artifacts.linux_aarch64'

echo "engine_bump_test: a bad date is refused before anything is read"
make_tree "$WORK/date"
write_payload "$WORK/payload.json"
if python3 "$SCRIPT" --payload "$WORK/payload.json" --root "$WORK/date" \
  --date "yesterday" >/dev/null 2>&1; then
  fail "a date that is not YYYY-MM-DD was accepted"
fi
echo "  refused: a date that is not YYYY-MM-DD"

echo "engine_bump_test: --check writes nothing"
make_tree "$WORK/check"
write_payload "$WORK/payload.json"
python3 "$SCRIPT" --payload "$WORK/payload.json" --root "$WORK/check" --check >/dev/null ||
  fail "a payload the engine actually sends was refused"
grep -q '"tag": null' "$WORK/check/engine/PIN.json" ||
  fail "--check wrote the pin"
grep -q 'version = "0.0.1"' "$WORK/check/App/Fermix/Cargo.toml" ||
  fail "--check wrote the crate version"
echo "  ok: validated, and nothing on disk moved"

echo "engine_bump_test: the bump it writes"
make_tree "$WORK/bump"
write_payload "$WORK/payload.json"
answer="$(python3 "$SCRIPT" --payload "$WORK/payload.json" --root "$WORK/bump" \
  --date 2026-09-19)" || fail "the bump refused a payload the engine actually sends"

for needed in "\"version\": \"$VERSION\"" "\"branch\": \"engine/$TAG\"" \
  "\"subject\": \"engine: pin $TAG\""; do
  case "$answer" in
    *"$needed"*) ;;
    *) fail "the bump does not report $needed" ;;
  esac
done

pin="$WORK/bump/engine/PIN.json"
for needed in "\"tag\": \"$TAG\"" "\"engine_version\": \"$VERSION\"" \
  "\"source_commit\": \"$COMMIT\"" "\"$DIGEST_X86\"" "\"$DIGEST_ARM\"" \
  "fermix_app_engine_linux_aarch64.tar.gz"; do
  grep -qF -- "$needed" "$pin" || fail "the written pin does not carry $needed"
done
# The note belongs to an unpinned pin, and a filled one that kept it would tell
# a reader to fill in a pin that is already filled.
if grep -q '"note"' "$pin"; then
  fail "the written pin kept the unpinned note"
fi

grep -q "^version = \"$VERSION\"$" "$WORK/bump/App/Fermix/Cargo.toml" ||
  fail "the crate version was not written"
grep -q "^version = \"$VERSION\"$" "$WORK/bump/App/Fermix/Cargo.lock" ||
  fail "the lock file's entry for the crate was not written"
grep -qF "<release version=\"$VERSION\" date=\"2026-09-19\"" \
  "$WORK/bump/packaging/io.tezra.Fermix.metainfo.xml" ||
  fail "the metainfo release entry was not written"
echo "  ok: the pin, the crate, the lock file and the metainfo carry one version"

echo "engine_bump_test: the pin it writes is one engine_pin.sh accepts"
# The whole point of writing the pin is that the release rail can read it. A pin
# this script wrote that engine_pin.sh refuses would fail minutes later, in a
# job whose error names neither this script nor the dispatch.
state="$(bash -c 'source "$1"; engine_pin_state "$2"' engine_bump_test \
  "$ROOT_DIR/scripts/engine_pin.sh" "$pin")" ||
  fail "engine_pin.sh refuses the pin this script wrote"
[ "$state" = "pinned" ] || fail "engine_pin.sh reads the written pin as $state"
echo "  ok: engine_pin.sh reads it as pinned"

echo "engine_bump_test: the bump is idempotent"
# A re-dispatch of the same tag updates the same branch, so running the bump
# twice over its own output has to produce the same tree rather than refusing on
# a version line it already wrote.
python3 "$SCRIPT" --payload "$WORK/payload.json" --root "$WORK/bump" \
  --date 2026-09-19 >/dev/null ||
  fail "a second run over its own output refused"
grep -q "^version = \"$VERSION\"$" "$WORK/bump/App/Fermix/Cargo.toml" ||
  fail "the second run changed the crate version"
echo "  ok: a re-dispatch of the same tag writes the same tree"

echo "engine_bump_test: every refusal fired"
