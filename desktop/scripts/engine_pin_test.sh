#!/usr/bin/env bash
# Offline tests for engine_pin.py: the writer against a stub gh serving a fixture release, and every
# refusal of the reader. Nothing here reaches the network.
#   desktop/scripts/engine_pin_test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/fixtures/engine"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export PATH="$fixtures/bin:$PATH"

COMMIT=0123456789abcdef0123456789abcdef01234567
IDENTITY=https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/v0.0.1
TAG_REF=api/repos/tezra-io/fermix/git/ref/tags/v0.0.1.json
TAG_OBJECT=api/repos/tezra-io/fermix/git/tags/fedcba9876543210fedcba9876543210fedcba98.json
RELEASE_JSON=api/repos/tezra-io/fermix/releases/tags/v0.0.1.json

fail() {
  echo "engine_pin_test: $*" >&2
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

expect_equal() {
  [ "$2" = "$3" ] || fail "$1 is '$2', expected '$3'"
}

new_release() {
  "$fixtures/fixture_release.sh" "$work/$1" "$fixtures/fermix_0.0.1_amd64.deb" \
    "$fixtures/fermix-0.0.1-1.x86_64.rpm" "$COMMIT" v0.0.1
}

# Applies one Python statement to a JSON file in place; the document is `doc`.
edit_json() {
  python3 - "$1" "$2" <<'PY'
import json
import sys

path, statement = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as handle:
    doc = json.load(handle)
exec(statement)
with open(path, "w", encoding="utf-8") as handle:
    json.dump(doc, handle, indent=2)
PY
}

# Removes one asset from a fixture release, from its files and from GitHub's answer.
drop_asset() {
  rm "$work/$1/assets/$2"
  edit_json "$work/$1/$RELEASE_JSON" "doc['assets'] = [a for a in doc['assets'] if a['name'] != '$2']"
}

pin_field() {
  "$here/engine_pin.py" --pin "$1" --get "$2"
}

# The writer refuses the release named by <name>, and the pin it was asked to replace stays as it was.
refuse_release() {
  local what="$1" reason="$2" name="$3"
  expect_refusal "$what" "$reason" \
    env FIXTURE_GH_RELEASE="$work/$name" "$here/engine_pin.py" --pin "$work/kept.json" v0.0.1
  cmp -s "$work/pin.json" "$work/kept.json" || fail "a refused pin ($what) changed the pin on disk"
}

# The reader refuses the good pin once one Python statement has broken it.
refuse_pin() {
  local what="$1" reason="$2" statement="$3" path="$work/broken.json"
  cp "$work/pin.json" "$path"
  edit_json "$path" "$statement"
  expect_refusal "$what" "$reason" "$here/engine_pin.py" --pin "$path" --check
  expect_refusal "$what, read for one field" "$reason" "$here/engine_pin.py" --pin "$path" --get tag
}

echo "engine_pin_test: writing the pin of a complete release"
new_release good
FIXTURE_GH_RELEASE="$work/good" "$here/engine_pin.py" --pin "$work/pin.json" v0.0.1 > /dev/null
"$here/engine_pin.py" --pin "$work/pin.json" --check
expect_equal repository "$(pin_field "$work/pin.json" repository)" tezra-io/fermix
expect_equal tag "$(pin_field "$work/pin.json" tag)" v0.0.1
expect_equal engine_version "$(pin_field "$work/pin.json" engine_version)" 0.0.1
expect_equal "the annotated tag's commit" "$(pin_field "$work/pin.json" source_commit)" "$COMMIT"
expect_equal certificate_identity "$(pin_field "$work/pin.json" certificate_identity)" "$IDENTITY"
expect_equal certificate_oidc_issuer "$(pin_field "$work/pin.json" certificate_oidc_issuer)" \
  https://token.actions.githubusercontent.com
expect_equal amd64.deb.asset "$(pin_field "$work/pin.json" amd64.deb.asset)" fermix_0.0.1_amd64.deb
expect_equal amd64.rpm.asset "$(pin_field "$work/pin.json" amd64.rpm.asset)" fermix-0.0.1-1.x86_64.rpm
expect_equal arm64.deb.asset "$(pin_field "$work/pin.json" arm64.deb.asset)" fermix_0.0.1_arm64.deb
expect_equal arm64.rpm.asset "$(pin_field "$work/pin.json" arm64.rpm.asset)" fermix-0.0.1-1.aarch64.rpm
for package in amd64.deb:fermix_0.0.1_amd64.deb amd64.rpm:fermix-0.0.1-1.x86_64.rpm \
  arm64.deb:fermix_0.0.1_arm64.deb arm64.rpm:fermix-0.0.1-1.aarch64.rpm; do
  expect_equal "${package%%:*}.sha256" "$(pin_field "$work/pin.json" "${package%%:*}.sha256")" \
    "$(sha256sum "$work/good/assets/${package#*:}" | cut -d' ' -f1)"
done
echo "  ok: every field, from the release's sidecars and the tag's commit"

echo "engine_pin_test: a lightweight tag names its commit directly"
new_release lightweight
printf '{"object": {"type": "commit", "sha": "%s"}}\n' "$COMMIT" > "$work/lightweight/$TAG_REF"
FIXTURE_GH_RELEASE="$work/lightweight" "$here/engine_pin.py" --pin "$work/lightweight.json" v0.0.1 > /dev/null
expect_equal "the lightweight tag's commit" "$(pin_field "$work/lightweight.json" source_commit)" "$COMMIT"
echo "  ok: $COMMIT"

echo "engine_pin_test: the writer refuses a release it cannot pin whole"
cp "$work/pin.json" "$work/kept.json"

new_release no-package
drop_asset no-package fermix-0.0.1-1.aarch64.rpm
refuse_release "a release missing the arm64 rpm" "fermix-0.0.1-1.aarch64.rpm" no-package

new_release no-sidecar
drop_asset no-sidecar fermix_0.0.1_amd64.deb.pem
refuse_release "a release missing a .pem" "fermix_0.0.1_amd64.deb.pem" no-sidecar

new_release no-sha256
drop_asset no-sha256 fermix_0.0.1_arm64.deb.sha256
refuse_release "a release missing a .sha256" "fermix_0.0.1_arm64.deb.sha256" no-sha256

new_release lying-sidecar
printf '%064d  fermix-0.0.1-1.x86_64.rpm\n' 0 > "$work/lying-sidecar/assets/fermix-0.0.1-1.x86_64.rpm.sha256"
refuse_release "a sidecar GitHub's digest disagrees with" "GitHub" lying-sidecar

new_release misnamed-sidecar
sed -i 's/fermix_0.0.1_amd64.deb/fermix_0.0.1_arm64.deb/' "$work/misnamed-sidecar/assets/fermix_0.0.1_amd64.deb.sha256"
refuse_release "a sidecar naming another file" "names fermix_0.0.1_arm64.deb" misnamed-sidecar

new_release no-digest
edit_json "$work/no-digest/$RELEASE_JSON" "doc['assets'][0]['digest'] = None"
refuse_release "a release asset GitHub records no digest for" "no sha256 digest" no-digest

new_release draft
edit_json "$work/draft/$RELEASE_JSON" "doc['draft'] = True"
refuse_release "a draft release" "draft" draft

new_release prerelease
edit_json "$work/prerelease/$RELEASE_JSON" "doc['prerelease'] = True"
refuse_release "a prerelease" "prerelease" prerelease

new_release endless-tag
printf '{"object": {"type": "tag", "sha": "fedcba9876543210fedcba9876543210fedcba98"}}\n' \
  > "$work/endless-tag/$TAG_OBJECT"
refuse_release "a tag that never reaches a commit" "does not reach a commit" endless-tag

new_release no-tag
rm "$work/no-tag/$TAG_REF"
refuse_release "a release whose tag GitHub cannot resolve" "v0.0.1" no-tag

expect_refusal "a tag that is not vX.Y.Z" "not an engine release tag" \
  env FIXTURE_GH_RELEASE="$work/good" "$here/engine_pin.py" --pin "$work/kept.json" 0.0.1
expect_refusal "a release GitHub does not have" "v0.0.2" \
  env FIXTURE_GH_RELEASE="$work/good" "$here/engine_pin.py" --pin "$work/kept.json" v0.0.2

echo "engine_pin_test: the reader refuses every pin that is not complete and consistent"
refuse_pin "a half-filled pin (a null digest)" "half filled" "doc['packages']['arm64']['rpm']['sha256'] = None"
refuse_pin "a half-filled pin (no commit)" "half filled" "del doc['source_commit']"
refuse_pin "a half-filled pin (an empty tag)" "half filled" "doc['tag'] = ''"
refuse_pin "a pin with no arm64" "half filled" "del doc['packages']['arm64']"
refuse_pin "a pin of another schema" "schema_version" "doc['schema_version'] = 1"
refuse_pin "a pin of a fork" "tezra-io/fermix" "doc['repository'] = 'someone/fermix'"
refuse_pin "a pin of another issuer" "issuer" "doc['certificate_oidc_issuer'] = 'https://accounts.google.com'"
refuse_pin "an identity for another tag" "certificate_identity" \
  "doc['certificate_identity'] = doc['certificate_identity'].replace('v0.0.1', 'v9.9.9')"
refuse_pin "a version that is not the tag's" "engine_version" "doc['engine_version'] = '0.0.2'"
refuse_pin "a short commit" "source_commit" "doc['source_commit'] = '0123456'"
refuse_pin "an asset the release does not publish" "fermix_0.0.1_x86_64.deb" \
  "doc['packages']['amd64']['deb']['asset'] = 'fermix_0.0.1_x86_64.deb'"
refuse_pin "a digest that is not sha256" "sha256" "doc['packages']['amd64']['rpm']['sha256'] = 'abc'"
refuse_pin "a key nobody reads" "unexpected" "doc['note'] = 'pinned by hand'"
refuse_pin "a third architecture" "unexpected" "doc['packages']['riscv64'] = doc['packages']['amd64']"

printf '{"schema_version": 3,' > "$work/truncated.json"
expect_refusal "a pin that is not JSON" "not valid JSON" "$here/engine_pin.py" --pin "$work/truncated.json" --check
expect_refusal "a pin that is not there" "cannot read" "$here/engine_pin.py" --pin "$work/absent.json" --check
expect_refusal "a field the pin does not have" "not a field" "$here/engine_pin.py" --pin "$work/pin.json" --get amd64.zip.asset
expect_refusal "no arguments" "usage" "$here/engine_pin.py"
expect_refusal "an unknown flag" "usage" "$here/engine_pin.py" --nonsense

echo "engine_pin_test: the checked-in pin"
"$here/engine_pin.py" --check
echo "  ok: desktop/engine/PIN.json pins $("$here/engine_pin.py" --get tag) completely"

echo "engine_pin_test: ok"
