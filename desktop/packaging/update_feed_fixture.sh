#!/usr/bin/env bash
#
# Writes the update feed fixture that update_feed.py and the app's reader are both tested against.
# update_feed.py makes it the way release.yml makes a feed: three releases, each written as an item,
# merged into the feed before it and signed, by the test key linux-feed-test. That key's seed is
# SHA-256 of the ASCII text "linux-feed-test". It signs nothing but fixtures, and update-keys.json
# never lists it.
#
#   update_feed_fixture.sh <dir>
#
# <dir> gets:
#   test-key-linux-feed-test.seed  the seed, as 64 lowercase hex characters
#   test-key-linux-feed-test.pem   the same key as PKCS#8 PEM, which openssl signs with
#   test-keys.json                 a keys file in update-keys.json's form, naming only that key
#   appcast.json                   the signed feed
#   expected-decode.json           what update_feed.py verify --decoded reads from it
# desktop/core/tests/fixtures/update-feed/ holds what this writes, beside its SOURCE.md.
# update_feed_test.sh writes it again and expects the same bytes, since ed25519 signs
# deterministically. The releases, newest first:
#   0.5.0  engine 0.18.0, minimum_installed_version 0.4.0
#   0.4.0  engine 0.17.0, critical
#   0.3.0  engine 0.16.0
# Each release's packages are under https://github.com/tezra-io/fermix-linux/releases/download/
# v<version>. Each package's bytes are the ASCII text "test package <file>" and a newline. Each
# engine pin names the source commit SHA-1("engine <version>").
set -euo pipefail
shopt -s inherit_errexit

KEY_ID=linux-feed-test
# PKCS#8's DER around an ed25519 seed.
PKCS8_PREFIX=302e020100300506032b657004220420
BASE_URL=https://github.com/tezra-io/fermix-linux/releases/download
here=$(cd "$(dirname "$0")" && pwd)
feed="$here/update_feed.py"

# What the EXIT trap removes: it runs after every function has returned.
WORK=""

fail() {
  echo "update_feed_fixture: $*" >&2
  exit 1
}

cleanup() {
  [ -z "$WORK" ] || rm -rf -- "$WORK"
}

sha256_of_text() {
  printf '%s' "$1" | sha256sum | cut -c1-64
}

# The seed, the key as PEM, and the keys file that names its public key.
write_key() {
  local out="$1" seed pem public
  seed="$out/test-key-$KEY_ID.seed"
  pem="$out/test-key-$KEY_ID.pem"
  sha256_of_text "$KEY_ID" > "$seed"
  # shellcheck disable=SC2016 # Python's text, not the shell's
  python3 -I -c 'import base64, sys
der = bytes.fromhex(sys.argv[1] + open(sys.argv[2]).read().strip())
sys.stdout.write("-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(der).decode() + "\n-----END PRIVATE KEY-----\n")' \
    "$PKCS8_PREFIX" "$seed" > "$pem"
  public="$(openssl pkey -in "$pem" -pubout -outform DER | tail -c 32 | base64 -w 0)"
  jq -n --arg id "$KEY_ID" --arg public "$public" \
    '{format: 1, keys: [{key_id: $id, role: "current", public_key: $public}]}' > "$out/test-keys.json"
}

# An engine pin engine_pin.py --check accepts, for <engine version>, its digests made up.
write_pin() {
  local file="$1" engine="$2"
  jq -n --arg version "$engine" --arg commit "$(printf 'engine %s' "$engine" | sha1sum | cut -c1-40)" \
    --arg amd64_deb "$(sha256_of_text "fermix_${engine}_amd64.deb")" \
    --arg amd64_rpm "$(sha256_of_text "fermix-${engine}-1.x86_64.rpm")" \
    --arg arm64_deb "$(sha256_of_text "fermix_${engine}_arm64.deb")" \
    --arg arm64_rpm "$(sha256_of_text "fermix-${engine}-1.aarch64.rpm")" '{
      schema_version: 3, repository: "tezra-io/fermix", tag: ("v" + $version),
      engine_version: $version, source_commit: $commit,
      certificate_oidc_issuer: "https://token.actions.githubusercontent.com",
      certificate_identity: ("https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/v" + $version),
      packages: {
        amd64: {deb: {asset: "fermix_\($version)_amd64.deb", sha256: $amd64_deb},
                rpm: {asset: "fermix-\($version)-1.x86_64.rpm", sha256: $amd64_rpm}},
        arm64: {deb: {asset: "fermix_\($version)_arm64.deb", sha256: $arm64_deb},
                rpm: {asset: "fermix-\($version)-1.aarch64.rpm", sha256: $arm64_rpm}}}}' > "$file"
}

write_packages() {
  local dir="$1" version="$2" name
  for name in "fermix-desktop_${version}_amd64.deb" "fermix-desktop-${version}-1.x86_64.rpm" \
    "fermix-desktop_${version}_arm64.deb" "fermix-desktop-${version}-1.aarch64.rpm"; do
    printf 'test package %s\n' "$name" > "$dir/$name"
  done
}

write_notes() {
  local file="$1" version="$2"
  case "$version" in
    0.3.0)
      cat > "$file" << 'EOF'
- Fermix plays the voice notes a phone sends, as a waveform in the chat.
- Run `fermix status` to see which engine is running.
EOF
      ;;
    0.4.0)
      cat > "$file" << 'EOF'
---
critical: true
---
- Fermix no longer stops answering when the tray menu opens twice.
EOF
      ;;
    0.5.0)
      cat > "$file" << 'EOF'
---
minimum_installed_version: 0.4.0
---
- Fermix checks for updates and installs them from the tray.
- Settings → Updates shows the version you have and when Fermix last checked.
- Places keep their accents, so a café reads as it should.
EOF
      ;;
    *) fail "the fixture has no notes for $version" ;;
  esac
}

# <version> written as an item, merged into the feed so far, and signed over it.
release() {
  local keys="$1" pem="$2" version="$3" engine="$4" published="$5" generated="$6" dir
  local -a previous=()
  dir="$WORK/$version"
  mkdir -p "$dir/packages"
  write_packages "$dir/packages" "$version"
  write_pin "$dir/PIN.json" "$engine"
  write_notes "$dir/notes.md" "$version"
  "$feed" item --packages "$dir/packages" --version "$version" --pin "$dir/PIN.json" \
    --notes "$dir/notes.md" --base-url "$BASE_URL/v$version" --published-at "$published" \
    --out "$dir/appcast-item.json" > /dev/null
  [ ! -f "$WORK/appcast.json" ] || previous=(--previous "$WORK/appcast.json")
  "$feed" merge --item "$dir/appcast-item.json" "${previous[@]}" --keys "$keys" \
    --generated-at "$generated" --out "$dir/payload.json" > /dev/null
  "$feed" sign --payload "$dir/payload.json" --keys "$keys" --out "$WORK/appcast.json" < "$pem" > /dev/null
}

main() {
  [ $# -eq 1 ] || fail "usage: update_feed_fixture.sh <dir>"
  local out="$1" keys pem
  [ -d "$out" ] || fail "there is no directory $out"
  keys="$out/test-keys.json"
  pem="$out/test-key-$KEY_ID.pem"
  WORK="$(mktemp -d)"
  write_key "$out"
  release "$keys" "$pem" 0.3.0 0.16.0 2026-10-23T18:00:00Z 2026-10-23T18:05:00Z
  release "$keys" "$pem" 0.4.0 0.17.0 2026-11-06T18:00:00Z 2026-11-06T18:05:00Z
  release "$keys" "$pem" 0.5.0 0.18.0 2026-11-20T18:00:00Z 2026-11-20T18:05:00Z
  cp -- "$WORK/appcast.json" "$out/appcast.json"
  "$feed" verify --feed "$out/appcast.json" --keys "$keys" --decoded "$out/expected-decode.json"
}

trap cleanup EXIT
main "$@"
