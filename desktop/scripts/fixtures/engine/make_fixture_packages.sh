#!/usr/bin/env bash
# Rebuilds the fixture engine packages the offline tests unpack, fermix_0.0.1_amd64.deb and
# fermix-0.0.1-1.x86_64.rpm, from nfpm.yaml with the nFPM release the engine's own packages are built
# with (NFPM_VERSION and NFPM_RELEASES in tezra-io/fermix scripts/release/linux_packages.py), and two
# rpms that hold the same files but declare what only an rpm can: requires/ has a Requires, and
# preinstall/ has a %pre scriptlet. It needs the network and an x86_64 host; the tests never run it,
# they read the checked-in result.
#   desktop/scripts/fixtures/engine/make_fixture_packages.sh
set -euo pipefail
NFPM_VERSION=2.47.0
NFPM_ASSET="nfpm_${NFPM_VERSION}_Linux_x86_64.tar.gz"
NFPM_SHA256=0660ca602b2d2d2ae4781a06c692b3eeb9d437ffea05b831d76e41f4a3188783

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL --retry 3 -o "$work/$NFPM_ASSET" \
  "https://github.com/goreleaser/nfpm/releases/download/v$NFPM_VERSION/$NFPM_ASSET"
echo "$NFPM_SHA256  $work/$NFPM_ASSET" | sha256sum --check --quiet -
tar -xzf "$work/$NFPM_ASSET" -C "$work" nfpm

cd "$here"
"$work/nfpm" package --config nfpm.yaml --packager deb --target fermix_0.0.1_amd64.deb
"$work/nfpm" package --config nfpm.yaml --packager rpm --target fermix-0.0.1-1.x86_64.rpm

sed 's|^scripts:$|depends:\n  - /bin/sh\n  - libc.so.6()(64bit)\n\nscripts:|' nfpm.yaml > "$work/requires.yaml"
sed 's|^  postremove: \(.*\)$|  postremove: \1\n  preinstall: \1|' nfpm.yaml > "$work/preinstall.yaml"
mkdir -p requires preinstall
"$work/nfpm" package --config "$work/requires.yaml" --packager rpm --target requires/fermix-0.0.1-1.x86_64.rpm
"$work/nfpm" package --config "$work/preinstall.yaml" --packager rpm --target preinstall/fermix-0.0.1-1.x86_64.rpm
