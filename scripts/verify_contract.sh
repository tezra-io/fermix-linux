#!/usr/bin/env bash
#
# Verify the vendored wire contracts.
#
# The canonical sources are `FermixCore.Management.Protocol` (the daemon's
# management protocol) and `Fermix.CLI.MachineOutput` (the typed `--json` verbs
# of the packaged command line) in the fermix repo. This repo carries one
# vendored tree, `App/Fermix/contracts/`, holding one directory per contract:
# `management/` and `cli/`. It is compiled into the application and pinned by
# two records:
#
#   CHECKSUMS.txt  the digest of every vendored file, in `shasum -a 256 -c` form
#   SOURCE.json    where each file came from and what it hashed to upstream
#
# Three checks always run:
#
#   1. the vendored bytes match CHECKSUMS.txt
#   2. the tree and CHECKSUMS.txt list exactly the same files, so a file cannot
#      be added or dropped without the pin noticing
#   3. CHECKSUMS.txt and SOURCE.json agree, so regenerating the checksums over a
#      locally edited file no longer verifies clean
#
# Every one of them covers every contract SOURCE.json lists, so adding a
# contract to that file is what puts it under the pin.
#
# The fourth needs the upstream repository and is therefore explicit:
#
#   scripts/verify_contract.sh --source <path-to-fermix-checkout>
#
# byte-compares every vendored file of every contract against the path
# SOURCE.json records. It is the only check that can see upstream moving ahead
# of the vendored copy, and it is what a re-vendor must be verified with.
#
# A contract carrying `committed_upstream: false` was vendored from an
# uncommitted engine working tree, which pins bytes nobody else can retrieve.
# That is reported as a note here, the way the macOS repository's
# `verify_protocol_contract.sh` reports it, and refused outright under:
#
#   scripts/verify_contract.sh --release
#
# which is what the release rail runs. A release audience that accepted such a
# pin would ship an application built against a contract that exists on one
# machine.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONTRACTS_DIR="${FERMIX_CONTRACTS_DIR:-$ROOT_DIR/App/Fermix/contracts}"
SOURCE_CHECKOUT=""
RELEASE=0

fail() {
  echo "verify_contract: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --source)
      [ $# -ge 2 ] || fail "--source needs a path to a fermix checkout"
      SOURCE_CHECKOUT="$2"
      shift 2
      ;;
    --release)
      RELEASE=1
      shift
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[ -d "$CONTRACTS_DIR" ] || fail "vendored contract tree is missing at $CONTRACTS_DIR"

cd "$CONTRACTS_DIR"

# 1. The vendored bytes are the pinned bytes.
if command -v shasum >/dev/null 2>&1; then
  shasum -a 256 -c CHECKSUMS.txt
else
  sha256sum -c CHECKSUMS.txt
fi

# 2. The manifest covers the tree exactly.
present="$(find . -type f ! -name CHECKSUMS.txt ! -name SOURCE.json |
  sed 's|^\./||' | LC_ALL=C sort)"
pinned="$(awk '{ print $2 }' CHECKSUMS.txt | LC_ALL=C sort)"
if [ "$present" != "$pinned" ]; then
  echo "vendored files:" >&2
  diff <(echo "$pinned") <(echo "$present") >&2 || true
  fail "CHECKSUMS.txt does not list exactly the files in the tree"
fi

# 3. The two records agree, file for file and digest for digest.
python3 - "$CONTRACTS_DIR" "$RELEASE" <<'PY'
import json, sys, pathlib

root = pathlib.Path(sys.argv[1])
provenance = json.loads((root / "SOURCE.json").read_text())

pinned = {}
for line in (root / "CHECKSUMS.txt").read_text().splitlines():
    if not line.strip():
        continue
    digest, path = line.split()
    pinned[path] = digest

recorded = {}
for contract in provenance["contracts"]:
    for entry in contract["files"]:
        recorded[entry["path"]] = entry["sha256"]

problems = []
for path in sorted(set(pinned) | set(recorded)):
    if path not in pinned:
        problems.append(f"{path}: in SOURCE.json but not in CHECKSUMS.txt")
    elif path not in recorded:
        problems.append(f"{path}: in CHECKSUMS.txt but not in SOURCE.json")
    elif pinned[path] != recorded[path]:
        problems.append(
            f"{path}: CHECKSUMS.txt has {pinned[path][:12]}, "
            f"SOURCE.json records {recorded[path][:12]}"
        )

if problems:
    for problem in problems:
        print(f"verify_contract: {problem}", file=sys.stderr)
    sys.exit(1)

drafts = [c["name"] for c in provenance["contracts"] if c.get("draft", False)]
if drafts:
    for name in drafts:
        print(
            f"verify_contract: the {name} contract declares itself a draft authored "
            f"from the design; a draft has no upstream to compare against, so it is "
            f"not shippable. Re-vendor it from the engine.",
            file=sys.stderr,
        )
    sys.exit(1)

uncommitted = [
    contract["name"]
    for contract in provenance["contracts"]
    if not contract.get("committed_upstream", True)
]
release = sys.argv[2] == "1"

for name in uncommitted:
    print(
        f"verify_contract: note: the {name} contract was vendored from "
        f"an uncommitted upstream working tree "
        f"({provenance['upstream']['commit'][:12]}); re-take the pin from the "
        f"commit that publishes it before release"
    )

if release and uncommitted:
    for name in uncommitted:
        print(
            f"verify_contract: the {name} contract was vendored from an uncommitted "
            f"upstream working tree, so the bytes it pins exist on one machine. "
            f"Re-vendor it from the engine commit that publishes the directory "
            f"before releasing.",
            file=sys.stderr,
        )
    sys.exit(1)
PY

echo "vendored wire contracts: checksums and provenance OK"

# 4. Optional, explicit: compare against the upstream checkout itself.
[ -n "$SOURCE_CHECKOUT" ] || exit 0
[ -d "$SOURCE_CHECKOUT" ] || fail "fermix checkout not found at $SOURCE_CHECKOUT"

python3 - "$CONTRACTS_DIR" "$SOURCE_CHECKOUT" <<'PY'
import json, sys, pathlib

root = pathlib.Path(sys.argv[1])
upstream = pathlib.Path(sys.argv[2])
provenance = json.loads((root / "SOURCE.json").read_text())

drift = []
for contract in provenance["contracts"]:
    for entry in contract["files"]:
        source = upstream / entry["source_path"]
        if not source.is_file():
            drift.append(f"{entry['source_path']}: missing from the upstream checkout")
        elif source.read_bytes() != (root / entry["path"]).read_bytes():
            drift.append(f"{entry['path']}: differs from {entry['source_path']}")

if drift:
    for line in drift:
        print(f"verify_contract: {line}", file=sys.stderr)
    print(
        "verify_contract: re-vendor the contract tree and regenerate CHECKSUMS.txt "
        "and SOURCE.json in the same change",
        file=sys.stderr,
    )
    sys.exit(1)
PY

pinned_commit="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["upstream"]["commit"])' \
  "$CONTRACTS_DIR/SOURCE.json")"
head_commit="$(git -C "$SOURCE_CHECKOUT" rev-parse HEAD)"
if [ "$pinned_commit" != "$head_commit" ]; then
  echo "verify_contract: note: the checkout is at ${head_commit:0:12}," \
    "the pin records ${pinned_commit:0:12}; the bytes match, so update the pin" \
    "when you next re-vendor"
fi

echo "vendored wire contracts: byte-identical to $SOURCE_CHECKOUT"
