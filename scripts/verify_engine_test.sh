#!/usr/bin/env bash
#
# Exercise the engine pin's readers and verify_engine.sh's refusals, offline.
#
# The pin exists to stop a package from carrying an engine nobody checked, and
# every one of its failure modes is quiet: a half-filled pin reads as a pin, a
# wrong digest reads as a download, an unsigned archive reads as an archive, a
# tar that writes outside its root reads as an unpack. So each refusal is driven
# here, against fixture archives this test builds from tiny fake trees, a
# fixture pin, and a stub cosign that answers what the case needs.
#
# Nothing here reaches the network, the real pin's engine, or a real cosign.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/verify-engine-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

TAG="v0.11.0"
VERSION="0.11.0"
COMMIT="1111111111111111111111111111111111111111"
IDENTITY="https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/$TAG"
BUILD_ID="release-1234567890"
TARGETS=(linux_x86_64 linux_aarch64)

FIXTURE="$ROOT_DIR/scripts/fixtures/engine/make_engine_archive.py"
STAGING="$WORK/staging"

fail() {
  echo "verify_engine_test: $*" >&2
  exit 1
}

digest_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# A cosign that always says yes, and one that always says no. The signature
# check and the digest check are separate refusals, and a test that could not
# tell them apart would pass while one of them was broken.
stub_cosign() {
  local path="$WORK/cosign-$1"
  {
    printf '#!/bin/sh\n'
    printf 'exit %s\n' "$2"
  } > "$path"
  chmod +x "$path"
  printf '%s\n' "$path"
}

ACCEPTS="$(stub_cosign accepts 0)"
REFUSES="$(stub_cosign refuses 1)"

asset_name() {
  printf 'fermix_app_engine_%s.tar.gz\n' "$1"
}

sidecars_for() {
  local path="$1"
  digest_of "$path" > "$path.sha256"
  printf 'signature\n' > "$path.sig"
  printf 'certificate\n' > "$path.pem"
}

# One honest archive per target, each with its three sidecars, in a directory of
# this test's own. The pin is then written to agree with them.
make_assets() {
  local dir="$1" broken="${2:-none}" build_id="${3:-$BUILD_ID}" target path
  mkdir -p "$dir"
  for target in "${TARGETS[@]}"; do
    path="$dir/$(asset_name "$target")"
    python3 "$FIXTURE" honest "$path" "$target" "$VERSION" "$COMMIT" "$IDENTITY" \
      "$build_id" "$broken" || fail "the fixture builder failed for $target/$broken"
    sidecars_for "$path"
  done
}

# The pin that matches those assets. The second argument writes one that does
# not.
write_pin() {
  local path="$1" assets="$2" broken="${3:-none}"
  python3 - "$path" "$assets" "$broken" "$TAG" "$VERSION" "$COMMIT" "$IDENTITY" <<'PY'
import hashlib
import json
import sys

path, assets, broken, tag, version, commit, identity = sys.argv[1:8]
TARGETS = ("linux_x86_64", "linux_aarch64")


def digest(target):
    with open(f"{assets}/fermix_app_engine_{target}.tar.gz", "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


pin = {
    "schema_version": 2,
    "repository": "tezra-io/fermix",
    "certificate_oidc_issuer": "https://token.actions.githubusercontent.com",
    "tag": tag,
    "engine_version": version,
    "source_commit": commit,
    "certificate_identity": identity,
    "artifacts": {
        target: {
            "asset": f"fermix_app_engine_{target}.tar.gz",
            "sha256": digest(target),
        }
        for target in TARGETS
    },
    "note": "a fixture pin, written by scripts/verify_engine_test.sh",
}

if broken == "half":
    pin["artifacts"]["linux_aarch64"]["sha256"] = None
elif broken == "tag":
    pin["tag"] = "0.11.0"
elif broken == "engine_version":
    pin["engine_version"] = "0.11.1"
elif broken == "commit":
    pin["source_commit"] = "1111"
elif broken == "identity":
    pin["certificate_identity"] = identity.replace(tag, "v9.9.9")
elif broken == "asset":
    pin["artifacts"]["linux_x86_64"]["asset"] = "fermix_app_engine_x86_64.tar.gz"
elif broken == "digest":
    pin["artifacts"]["linux_x86_64"]["sha256"] = "0" * 64
elif broken == "digestformat":
    pin["artifacts"]["linux_x86_64"]["sha256"] = "not a digest"
elif broken == "schema":
    pin["schema_version"] = 1
elif broken == "targets":
    pin["artifacts"]["linux_riscv64"] = dict(pin["artifacts"]["linux_x86_64"])
elif broken == "packages":
    pin["packages"] = pin.pop("artifacts")
elif broken == "unpinned_without_note":
    del pin["note"]
    broken = "unpinned"
elif broken == "commit_mismatch":
    pin["source_commit"] = "2" * 40

if broken == "unpinned":
    for field in ("tag", "engine_version", "source_commit", "certificate_identity"):
        pin[field] = None
    for artifact in pin["artifacts"].values():
        artifact["asset"] = None
        artifact["sha256"] = None

with open(path, "w", encoding="utf-8") as handle:
    json.dump(pin, handle, indent=2)
PY
}

reset_staging() {
  rm -rf "$STAGING"
  mkdir -p "$STAGING"
}

staging_is_empty() {
  [ -z "$(find "$STAGING" -mindepth 1 -print -quit)" ]
}

# A refusal is only proved when it is the refusal that was meant. Every case
# names the sentence it expects, because a script that refused for another
# reason entirely, a missing sidecar say, would otherwise pass every case here
# while the check it is named after was never reached.
expect_refusal() {
  local what="$1" expected="$2" output
  shift 2
  reset_staging
  if output="$("$@" 2>&1)"; then
    fail "$what was accepted"
  fi
  case "$output" in
    *"$expected"*) ;;
    *) fail "$what was refused, and not for its own reason. Wanted '$expected', got: $output" ;;
  esac
  staging_is_empty || fail "$what was refused and left files in the staging directory"
  echo "  refused: $what"
}

expect_acceptance() {
  local what="$1"
  shift
  reset_staging
  "$@" >/dev/null || fail "$what was refused"
  echo "  accepted: $what"
}

verify() {
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$@"
}

echo "verify_engine_test: the scripts parse"
for script in engine_pin.sh fetch_engine.sh verify_engine.sh; do
  bash -n "$ROOT_DIR/scripts/$script" || fail "scripts/$script does not parse"
done
echo "  ok: shell syntax"

echo "verify_engine_test: the pin this repository ships"
STATE="$(bash -c "source '$ROOT_DIR/scripts/engine_pin.sh'; engine_pin_state '$ROOT_DIR/engine/PIN.json'")" ||
  fail "the checked-in pin does not read"
[ "$STATE" = "unpinned" ] ||
  fail "the checked-in pin reports $STATE, and this slice ships it unpinned"
echo "  ok: unpinned, and it says so"

reset_staging
expect_refusal "verifying against an unpinned pin" \
  "is unpinned, so there is no engine release to verify against" \
  verify "$ROOT_DIR/engine/PIN.json" "$WORK" --staging "$STAGING" --cosign "$ACCEPTS"
expect_refusal "downloading against an unpinned pin" \
  "is unpinned, so there is no engine release to download" \
  bash "$ROOT_DIR/scripts/fetch_engine.sh" "$ROOT_DIR/engine/PIN.json" "$WORK/download"

echo "verify_engine_test: a filled pin and the archives it names"
ASSETS="$WORK/assets"
make_assets "$ASSETS"
write_pin "$WORK/pin.json" "$ASSETS"

expect_acceptance "two archives whose digests, signatures, manifests and trees agree with the pin" \
  verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --cosign "$ACCEPTS"

echo "verify_engine_test: what an accepted verification leaves behind"
reset_staging
verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --cosign "$ACCEPTS" >/dev/null
for target in "${TARGETS[@]}"; do
  [ -f "$STAGING/$target/fermix_app_engine/engine-manifest.json" ] ||
    fail "$target left no manifest in the staging directory"
  [ -x "$STAGING/$target/fermix_app_engine/tree/usr/bin/fermix" ] ||
    fail "$target left no executable engine in the staging directory"
  [ -L "$STAGING/$target/fermix_app_engine/tree/usr/lib/fermix/current" ] ||
    fail "$target left no symbolic link where the tree carries one"
done
echo "  ok: <staging>/<target>/fermix_app_engine/ per target"

echo "verify_engine_test: one target at a time"
reset_staging
verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --target linux_aarch64 \
  --cosign "$ACCEPTS" >/dev/null || fail "verifying one target was refused"
[ -d "$STAGING/linux_aarch64" ] || fail "the requested target was not unpacked"
[ ! -e "$STAGING/linux_x86_64" ] || fail "a target nobody asked for was unpacked"
echo "  ok: --target unpacks that target and no other"

echo "verify_engine_test: refusals in the record"
declare -A PIN_REFUSALS=(
  [half]="the engine pin is half filled"
  [tag]="is not an engine release tag"
  [engine_version]="the engine pin's engine_version is"
  [commit]="is not a 40-character commit"
  [identity]="signs as"
  [asset]="and the engine release publishes"
  [digest]="and the engine pin records"
  [digestformat]="is not a 64-character digest"
  [schema]="this reader understands 2"
  [targets]="must carry exactly the targets"
  [packages]="must carry exactly the targets"
  [unpinned_without_note]="carries no note saying how to fill it"
)
for broken in "${!PIN_REFUSALS[@]}"; do
  write_pin "$WORK/broken-$broken.json" "$ASSETS" "$broken"
  expect_refusal "a pin whose $broken is wrong" "${PIN_REFUSALS[$broken]}" \
    verify "$WORK/broken-$broken.json" "$ASSETS" --staging "$STAGING" --cosign "$ACCEPTS"
done

echo "verify_engine_test: refusals in what arrived"

TAMPERED="$WORK/tampered"
cp -R "$ASSETS" "$TAMPERED"
printf 'one more byte\n' >> "$TAMPERED/$(asset_name linux_aarch64)"
expect_refusal "an archive whose bytes changed after it was pinned" \
  "and the engine pin records" \
  verify "$WORK/pin.json" "$TAMPERED" --staging "$STAGING" --cosign "$ACCEPTS"

# The sidecar the release wrote agrees with the tampered archive, and the pin
# does not. This is the case the comment in verify_engine.sh is about.
digest_of "$TAMPERED/$(asset_name linux_aarch64)" > "$TAMPERED/$(asset_name linux_aarch64).sha256"
expect_refusal "an archive whose own sidecar agrees with it and the pin does not" \
  "and the engine pin records" \
  verify "$WORK/pin.json" "$TAMPERED" --staging "$STAGING" --cosign "$ACCEPTS"

UNSIGNED="$WORK/unsigned"
cp -R "$ASSETS" "$UNSIGNED"
rm "$UNSIGNED/$(asset_name linux_aarch64).sig"
expect_refusal "an archive with no signature beside it" \
  "has no fermix_app_engine_linux_aarch64.tar.gz.sig beside it" \
  verify "$WORK/pin.json" "$UNSIGNED" --staging "$STAGING" --cosign "$ACCEPTS"

NOCERT="$WORK/nocert"
cp -R "$ASSETS" "$NOCERT"
rm "$NOCERT/$(asset_name linux_x86_64).pem"
expect_refusal "an archive with no certificate beside it" \
  "has no fermix_app_engine_linux_x86_64.tar.gz.pem beside it" \
  verify "$WORK/pin.json" "$NOCERT" --staging "$STAGING" --cosign "$ACCEPTS"

expect_refusal "an archive whose signature does not verify" \
  "carries no signature from" \
  verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --cosign "$REFUSES"

INCOMPLETE="$WORK/incomplete"
mkdir -p "$INCOMPLETE"
cp "$ASSETS/$(asset_name linux_x86_64)"* "$INCOMPLETE/"
expect_refusal "a download that carries one target and not the other" \
  "is not in $INCOMPLETE" \
  verify "$WORK/pin.json" "$INCOMPLETE" --staging "$STAGING" --cosign "$ACCEPTS"

expect_refusal "a verifier that is not executable" \
  "the cosign binary is not executable" \
  verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --cosign "$WORK/no-such-cosign"

expect_refusal "an unknown argument" "unknown argument '--nonsense'" \
  verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --nonsense

expect_refusal "a target the engine does not publish" \
  "no engine archive is published for 'linux_riscv64'" \
  verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --target linux_riscv64

expect_refusal "no staging directory at all" "--staging <dir> is required" \
  verify "$WORK/pin.json" "$ASSETS" --cosign "$ACCEPTS"

echo "verify_engine_test: the staging directory is the verifier's to fill"
reset_staging
touch "$STAGING/left-over-from-an-earlier-run"
if verify "$WORK/pin.json" "$ASSETS" --staging "$STAGING" --cosign "$ACCEPTS" >/dev/null 2>&1; then
  fail "a staging directory that already held files was accepted"
fi
[ -f "$STAGING/left-over-from-an-earlier-run" ] ||
  fail "a refused verification removed a file it did not put there"
echo "  refused: a staging directory that already holds files"

echo "verify_engine_test: refusals in the manifest"
declare -A MANIFEST_REFUSALS=(
  [schema]="declares schema_version 2, and this reader understands 1"
  [commit]="the manifest's source_commit is"
  [version]="the manifest's product_version is"
  [distribution]="the manifest's distribution_identity is"
  [target]="the manifest's artifact_target is"
  [arch]="the manifest's architecture is"
  [identity]="the manifest's certificate_identity is"
  [digest]="and what was unpacked digests to"
  [nomanifest]="carries no engine-manifest.json at its root"
  [badjson]="engine-manifest.json is not readable JSON"
  [manifestlink]="carries no engine-manifest.json at its root"
)
for broken in "${!MANIFEST_REFUSALS[@]}"; do
  BROKEN_ASSETS="$WORK/manifest-$broken"
  make_assets "$BROKEN_ASSETS" "$broken"
  write_pin "$WORK/pin-manifest-$broken.json" "$BROKEN_ASSETS"
  expect_refusal "a manifest whose $broken is wrong" "${MANIFEST_REFUSALS[$broken]}" \
    verify "$WORK/pin-manifest-$broken.json" "$BROKEN_ASSETS" --staging "$STAGING" \
    --cosign "$ACCEPTS"
done

echo "verify_engine_test: refusals in the archive itself"
EVIL="$WORK/evil"
mkdir -p "$EVIL"
declare -A ARCHIVE_REFUSALS=(
  [root]="is not under fermix_app_engine/, and that is the only root"
  [absolute]="is an absolute path"
  [dotdot]="carries a '..' path component"
  [duplicate]="appears twice"
  [undersymlink]="is under the symbolic link"
  [symlinkescape]="points outside the archive root"
  [implicitdir]="has no directory entry for"
  [device]="is a device or a pipe"
  [hardlink]="is a hard link"
  [setuid]="carries a setuid or setgid mode"
  [setgid]="carries a setuid or setgid mode"
  [deep]="is deeper than 32 directories"
  [notgzip]="is not a readable tar.gz"
)
for kind in "${!ARCHIVE_REFUSALS[@]}"; do
  path="$EVIL/$kind-$(asset_name linux_x86_64)"
  python3 "$FIXTURE" evil "$path" "$kind" || fail "the fixture builder failed for $kind"
  sidecars_for "$path"
  expect_refusal "an archive carrying $kind" "${ARCHIVE_REFUSALS[$kind]}" \
    bash "$ROOT_DIR/scripts/verify_engine.sh" --local-archive "$path" --dev \
    --target linux_x86_64 --staging "$STAGING"
done

echo "verify_engine_test: the unpack caps"
DEV_ASSETS="$WORK/dev"
make_assets "$DEV_ASSETS" none "dev-local-1"
DEV_ARCHIVE="$DEV_ASSETS/$(asset_name linux_x86_64)"

# shellcheck disable=SC2120,SC2119
# The cap cases below do pass arguments to this, through expect_refusal, which
# is an indirection the linter cannot see across; the argument-free call further
# down is the deliberate one.
dev_verify() {
  bash "$ROOT_DIR/scripts/verify_engine.sh" --local-archive "$DEV_ARCHIVE" --dev \
    --target linux_x86_64 --staging "$STAGING" "$@"
}

expect_refusal "an archive with a file above the file cap" "is larger than 8 bytes" \
  dev_verify --max-file-bytes 8
expect_refusal "an archive above the total cap" "unpacks to more than 16 bytes" \
  dev_verify --max-total-bytes 16
expect_refusal "an archive with more entries than the cap" "more than 3 entries" \
  dev_verify --max-entries 3
expect_refusal "a cap raised above its default" "a cap may be lowered, never raised" \
  dev_verify --max-file-bytes 99999999999
expect_refusal "a cap that is not a number" "needs a whole number" \
  dev_verify --max-entries plenty

echo "verify_engine_test: the development mode"
expect_acceptance "an unsigned locally built archive with a development build id" dev_verify

reset_staging
DEV_OUTPUT="$(dev_verify 2>&1 >/dev/null)"
case "$DEV_OUTPUT" in
  *"NOT RELEASE GRADE"*) echo "  ok: it says it is not release grade" ;;
  *) fail "--dev did not say that its result is not release grade" ;;
esac

expect_refusal "a release build verified with --dev" \
  "is a release build, and --dev verifies no signature" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" --local-archive "$ASSETS/$(asset_name linux_x86_64)" \
  --dev --target linux_x86_64 --staging "$STAGING"

expect_refusal "--dev with no target" "exactly one --target is required" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" --local-archive "$DEV_ARCHIVE" --dev \
  --staging "$STAGING"

expect_refusal "--dev with both targets" "exactly one --target is required" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" --local-archive "$DEV_ARCHIVE" --dev \
  --target linux_x86_64 --target linux_aarch64 --staging "$STAGING"

expect_refusal "--dev alongside a pin and a download directory" \
  "takes no pin and no download directory" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$ASSETS" --dev \
  --local-archive "$DEV_ARCHIVE" --target linux_x86_64 --staging "$STAGING"

expect_refusal "--local-archive without --dev" "only verifiable with --dev" \
  verify "$WORK/pin.json" "$ASSETS" --local-archive "$DEV_ARCHIVE" --staging "$STAGING" \
  --cosign "$ACCEPTS"

expect_refusal "--dev on a machine that says it is CI" \
  "a developer's shortcut past the signature, and CI=true" \
  env CI=true bash "$ROOT_DIR/scripts/verify_engine.sh" --local-archive "$DEV_ARCHIVE" \
  --dev --target linux_x86_64 --staging "$STAGING"

echo "verify_engine_test: downloading refuses before it reaches the network"
mkdir -p "$WORK/occupied"
touch "$WORK/occupied/left-over-from-an-earlier-run"
expect_refusal "a download directory that already holds files" \
  "the download directory already holds files" \
  bash "$ROOT_DIR/scripts/fetch_engine.sh" "$WORK/pin.json" "$WORK/occupied"
expect_refusal "a download of a target the engine does not publish" \
  "no engine archive is published for 'linux_riscv64'" \
  bash "$ROOT_DIR/scripts/fetch_engine.sh" "$WORK/pin.json" "$WORK/download" --target linux_riscv64
expect_refusal "a download with an unknown argument" "unknown argument '--nonsense'" \
  bash "$ROOT_DIR/scripts/fetch_engine.sh" "$WORK/pin.json" "$WORK/download" --nonsense

echo "verify_engine_test: every refusal fired"
