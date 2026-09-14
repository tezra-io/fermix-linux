#!/usr/bin/env bash
#
# Exercise check_vendor_marks.sh's refusals against throwaway copies of the
# marks tree. Every failure this gate exists for is silent on screen: a mark
# whose bytes changed still draws, a mark missing from the bundle draws the
# text treatment on every host, and a vendor the daemon publishes with no
# record draws its name. So the gate itself has to be seen failing.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/check-marks-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "check_vendor_marks_test: $*" >&2
  exit 1
}

# A repository holding only what the gate reads.
copy_repo() {
  local into="$WORK/$1"
  mkdir -p "$into/scripts" "$into/App/Fermix/resources" \
    "$into/App/Fermix/contracts/management/fixtures"
  cp "$ROOT_DIR/scripts/check_vendor_marks.sh" "$ROOT_DIR/scripts/vendor_marks.py" \
    "$into/scripts/"
  cp -R "$ROOT_DIR/App/Fermix/resources/VendorMarks" "$into/App/Fermix/resources/"
  cp "$ROOT_DIR/App/Fermix/resources/fermix.gresource.xml" "$into/App/Fermix/resources/"
  cp "$ROOT_DIR/App/Fermix/contracts/management/fixtures/success.jsonl" \
    "$into/App/Fermix/contracts/management/fixtures/"
  echo "$into"
}

expect_refusal() {
  local what="$1" repo="$2"
  shift 2
  if bash "$repo/scripts/check_vendor_marks.sh" "$@" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "check_vendor_marks_test: the repository as it stands"
"$ROOT_DIR/scripts/check_vendor_marks.sh" >/dev/null || fail "the real repository does not pass"
echo "  accepted: the repository"

echo "check_vendor_marks_test: refusals"

MARKS="App/Fermix/resources/VendorMarks"

repo="$(copy_repo changed-bytes)"
printf 'x' >>"$repo/$MARKS/providers/openai-color.svg"
expect_refusal "a mark whose bytes no longer hash to the record" "$repo"

repo="$(copy_repo wrong-format)"
cp "$repo/$MARKS/channels/whatsapp-color.webp" "$repo/$MARKS/providers/anthropic-color.png"
expect_refusal "a file whose bytes are not the format its name claims" "$repo"

repo="$(copy_repo orphan)"
cp "$repo/$MARKS/providers/openai-color.svg" "$repo/$MARKS/providers/nobody-claims-this.svg"
expect_refusal "a file in the tree that no record claims" "$repo"

repo="$(copy_repo missing-from-bundle)"
python3 - "$repo" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1]) / "App/Fermix/resources/fermix.gresource.xml"
body = path.read_text()
kept = [line for line in body.splitlines() if "providers/openai-color.svg" not in line]
path.write_text("\n".join(kept) + "\n")
PY
expect_refusal "a recorded mark the resource bundle does not serve" "$repo"

repo="$(copy_repo edited-roster)"
python3 - "$repo" <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]) / "App/Fermix/resources/VendorMarks/ROSTER.json"
roster = json.loads(path.read_text())
roster["providers"] = [p for p in roster["providers"] if p != "mistral"]
path.write_text(json.dumps(roster, indent=2))
PY
expect_refusal "the roster edited without re-pinning its hash" "$repo"

repo="$(copy_repo dropped-vendor)"
python3 - "$repo" <<'PY'
import hashlib, json, pathlib, sys
marks = pathlib.Path(sys.argv[1]) / "App/Fermix/resources/VendorMarks"
roster_path = marks / "ROSTER.json"
roster = json.loads(roster_path.read_text())
roster["providers"] = [p for p in roster["providers"] if p != "mistral"]
payload = json.dumps(roster, indent=2).encode()
roster_path.write_bytes(payload)

provenance_path = marks / "PROVENANCE.json"
record = json.loads(provenance_path.read_text())
record["roster_source"]["roster_sha256"] = hashlib.sha256(payload).hexdigest()
record["roster_source"]["provider_count"] = 6
record["marks"] = [m for m in record["marks"] if not (m["kind"] == "provider" and m["key"] == "mistral")]
provenance_path.write_text(json.dumps(record, indent=2))
for asset in marks.glob("providers/mistral-*"):
    asset.unlink()
PY
python3 - "$repo" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1]) / "App/Fermix/resources/fermix.gresource.xml"
kept = [line for line in path.read_text().splitlines() if "providers/mistral-color.svg" not in line]
path.write_text("\n".join(kept) + "\n")
PY
expect_refusal "a vendor dropped from both records while the daemon still publishes it" "$repo"

repo="$(copy_repo plugin-drift)"
mkdir -p "$WORK/fake-fermix/apps/fermix_core/priv/plugins"
cat >"$WORK/fake-fermix/apps/fermix_core/priv/plugins/index.json" <<'JSON'
{"plugins": [{"name": "a_plugin_nobody_recorded"}]}
JSON
cat >"$WORK/fake-fermix/apps/fermix_core/priv/plugins/catalog.json" <<'JSON'
{"plugins": []}
JSON
expect_refusal "a plugin the engine ships with no record" "$repo" --fermix-repo "$WORK/fake-fermix"

echo "check_vendor_marks_test: the skip names itself"
bash "$ROOT_DIR/scripts/check_vendor_marks.sh" 2>&1 |
  grep -qF "plugin-union check skipped" ||
  fail "the skipped plugin check does not say so"
echo "  ok: the skipped check is announced"
