#!/usr/bin/env bash
# Writes a fixture engine release v0.0.1 of tezra-io/fermix that the stub gh in bin/ serves: the
# eight packages with their .sha256, .sig and .pem sidecars under assets/, and under api/ the answers
# GitHub gives for the release and for the tag, which is annotated as the engine's tags are.
#
#   fixture_release.sh <release dir> <amd64 deb> <amd64 rpm> <tag commit> <signed as tag>
#
# The amd64 packages are the ones given; the arm64 ones are placeholder bytes nothing unpacks. Each
# signature is the stub cosign's: the .pem names the release workflow at <signed as tag>, so a tag
# other than v0.0.1 makes a release whose packages carry the wrong identity.
set -euo pipefail

USAGE="usage: fixture_release.sh <release dir> <amd64 deb> <amd64 rpm> <tag commit> <signed as tag>"
[ "$#" -eq 5 ] || {
  echo "$USAGE" >&2
  exit 2
}
release="$1" deb="$2" rpm="$3" commit="$4" signed_as="$5"
[ ! -e "$release" ] || {
  echo "fixture_release: $release already exists" >&2
  exit 1
}

TAG=v0.0.1
TAG_OBJECT=fedcba9876543210fedcba9876543210fedcba98
ISSUER=https://token.actions.githubusercontent.com
IDENTITY="https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/$signed_as"

assets="$release/assets"
mkdir -p "$assets" "$release/api/repos/tezra-io/fermix/releases/tags" \
  "$release/api/repos/tezra-io/fermix/git/ref/tags" "$release/api/repos/tezra-io/fermix/git/tags"

cp "$deb" "$assets/fermix_0.0.1_amd64.deb"
cp "$rpm" "$assets/fermix-0.0.1-1.x86_64.rpm"
printf 'arm64 placeholder\n' > "$assets/fermix_0.0.1_arm64.deb"
printf 'aarch64 placeholder\n' > "$assets/fermix-0.0.1-1.aarch64.rpm"

for name in fermix_0.0.1_amd64.deb fermix-0.0.1-1.x86_64.rpm fermix_0.0.1_arm64.deb \
  fermix-0.0.1-1.aarch64.rpm; do
  digest="$(sha256sum "$assets/$name" | cut -d' ' -f1)"
  printf '%s  %s\n' "$digest" "$name" > "$assets/$name.sha256"
  printf '%s' "$digest" > "$assets/$name.sig"
  printf '%s\n%s\n' "$IDENTITY" "$ISSUER" > "$assets/$name.pem"
done

printf '{"object": {"type": "tag", "sha": "%s"}}\n' "$TAG_OBJECT" \
  > "$release/api/repos/tezra-io/fermix/git/ref/tags/$TAG.json"
printf '{"object": {"type": "commit", "sha": "%s"}}\n' "$commit" \
  > "$release/api/repos/tezra-io/fermix/git/tags/$TAG_OBJECT.json"

python3 - "$assets" "$TAG" > "$release/api/repos/tezra-io/fermix/releases/tags/$TAG.json" <<'PY'
import hashlib
import json
import os
import sys

assets, tag = sys.argv[1], sys.argv[2]
entries = []
for name in sorted(os.listdir(assets)):
    with open(os.path.join(assets, name), "rb") as handle:
        content = handle.read()
    digest = hashlib.sha256(content).hexdigest()
    entries.append(
        {"name": name, "state": "uploaded", "size": len(content), "digest": f"sha256:{digest}"}
    )
release = {"tag_name": tag, "draft": False, "prerelease": False, "assets": entries}
print(json.dumps(release, indent=2))
PY
