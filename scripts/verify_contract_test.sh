#!/usr/bin/env bash
#
# Exercise verify_contract.sh's refusals against throwaway copies of the
# vendored tree. A gate nobody has seen fail is a gate nobody knows works.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/verify_contract.sh"
CONTRACTS="$ROOT_DIR/App/Fermix/contracts"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/verify-contract-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "verify_contract_test: $*" >&2
  exit 1
}

copy_tree() {
  local into="$WORK/$1"
  mkdir -p "$into"
  cp -R "$CONTRACTS/." "$into/"
  echo "$into"
}

expect_refusal() {
  local what="$1" tree="$2"
  if FERMIX_CONTRACTS_DIR="$tree" "$SCRIPT" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "verify_contract_test: the vendored tree as it stands"
"$SCRIPT" >/dev/null || fail "the real tree does not verify"
echo "  accepted: the vendored tree"

echo "verify_contract_test: refusals"

tree="$(copy_tree edited-bytes)"
printf '\n<!-- edited here -->\n' >> "$tree/management/PROTOCOL.md"
expect_refusal "a vendored file edited by hand" "$tree"

tree="$(copy_tree extra-file)"
echo '{}' > "$tree/management/extra.json"
expect_refusal "a file the manifest does not list" "$tree"

tree="$(copy_tree dropped-file)"
rm "$tree/management/fixtures/errors.jsonl"
expect_refusal "a pinned file that is gone" "$tree"

tree="$(copy_tree rehashed)"
printf '\n<!-- edited here -->\n' >> "$tree/management/PROTOCOL.md"
(
  cd "$tree"
  # xargs runs a program, never a shell function, so the tool is chosen as a
  # command name here rather than wrapped.
  if command -v shasum >/dev/null 2>&1; then
    set -- shasum -a 256
  else
    set -- sha256sum
  fi
  find . -type f ! -name CHECKSUMS.txt ! -name SOURCE.json |
    sed 's|^\./||' | LC_ALL=C sort | xargs "$@" > CHECKSUMS.txt
)
expect_refusal "checksums regenerated over a locally edited file" "$tree"

tree="$(copy_tree draft)"
python3 - "$tree" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
provenance = json.loads((root / "SOURCE.json").read_text())
provenance["contracts"][0]["draft"] = True
(root / "SOURCE.json").write_text(json.dumps(provenance, indent=2))
PY
expect_refusal "a contract that declares itself a draft" "$tree"

# Every check covers every contract SOURCE.json lists, and the second contract
# is the one a re-vendor is most likely to leave half done.
tree="$(copy_tree edited-cli)"
printf '\n<!-- edited here -->\n' >> "$tree/cli/CONTRACT.md"
expect_refusal "a vendored file of the second contract edited by hand" "$tree"

tree="$(copy_tree dropped-cli-golden)"
rm "$tree/cli/fixtures/service_status/active_aligned.json"
expect_refusal "a pinned golden of the second contract that is gone" "$tree"

tree="$(copy_tree unlisted-contract)"
python3 - "$tree" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
provenance = json.loads((root / "SOURCE.json").read_text())
provenance["contracts"] = [c for c in provenance["contracts"] if c["name"] != "cli"]
(root / "SOURCE.json").write_text(json.dumps(provenance, indent=2))
PY
expect_refusal "a vendored contract SOURCE.json stopped listing" "$tree"

echo "verify_contract_test: the release audience"

# An uncommitted upstream pin is a note to a developer and a refusal to the
# release rail, and both halves are asserted here: a note that also refused
# would stop every build, and a refusal that only noted would ship bytes that
# exist on one machine.
tree="$(copy_tree uncommitted)"
python3 - "$tree" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
provenance = json.loads((root / "SOURCE.json").read_text())
for contract in provenance["contracts"]:
    contract["committed_upstream"] = False
(root / "SOURCE.json").write_text(json.dumps(provenance, indent=2))
PY
note="$(FERMIX_CONTRACTS_DIR="$tree" "$SCRIPT")" ||
  fail "an uncommitted pin was refused outside the release audience"
case "$note" in
  *"uncommitted upstream working tree"*) ;;
  *) fail "an uncommitted pin was accepted without a note" ;;
esac
echo "  accepted with a note: an uncommitted upstream pin"

if FERMIX_CONTRACTS_DIR="$tree" "$SCRIPT" --release >/dev/null 2>&1; then
  fail "the release audience accepted an uncommitted upstream pin"
fi
echo "  refused: an uncommitted upstream pin, at release"

committed="$(copy_tree committed)"
python3 - "$committed" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
provenance = json.loads((root / "SOURCE.json").read_text())
for contract in provenance["contracts"]:
    contract["committed_upstream"] = True
(root / "SOURCE.json").write_text(json.dumps(provenance, indent=2))
PY
FERMIX_CONTRACTS_DIR="$committed" "$SCRIPT" --release >/dev/null ||
  fail "the release audience refused a fully committed pin"
echo "  accepted: a pin taken from committed engine bytes, at release"

echo "verify_contract_test: the upstream comparison"
UPSTREAM="$WORK/upstream"
# Built from the pin itself rather than from a hand-written file list, so a
# contract added to SOURCE.json is compared here without this harness being
# edited as well.
python3 - "$CONTRACTS" "$UPSTREAM" <<'PY'
import json, pathlib, shutil, sys

contracts = pathlib.Path(sys.argv[1])
upstream = pathlib.Path(sys.argv[2])
provenance = json.loads((contracts / "SOURCE.json").read_text())

for contract in provenance["contracts"]:
    for entry in contract["files"]:
        destination = upstream / entry["source_path"]
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(contracts / entry["path"], destination)
PY
git -C "$UPSTREAM" init -q
git -C "$UPSTREAM" add -A
git -C "$UPSTREAM" -c user.email=t@example.com -c user.name=test commit -qm "vendored"

"$SCRIPT" --source "$UPSTREAM" >/dev/null || fail "an identical upstream did not verify"
echo "  accepted: an upstream whose bytes match"

printf '\n<!-- upstream moved -->\n' >> "$UPSTREAM/apps/fermix_core/priv/cli/CONTRACT.md"
if "$SCRIPT" --source "$UPSTREAM" >/dev/null 2>&1; then
  fail "upstream moving ahead of the pin was accepted"
fi
echo "  refused: upstream moving ahead of the vendored copy"

echo "verify_contract_test: every refusal fired"
