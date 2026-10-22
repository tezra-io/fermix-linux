#!/usr/bin/env bash
# Offline tests for update_feed.py, the update feed's writer and checker. The shared fixture in
# desktop/core/tests/fixtures/update-feed is what update_feed_fixture.sh writes, byte for byte.
# OpenSSL signs RFC 8032's own Ed25519 test vectors. The production keys are the two the app trusts,
# and neither is the test key. Then each subcommand writes what the format says and refuses
# everything else, each refusal for its own reason. verify's refusals come from envelopes and
# payloads signed here directly with the test key, so verify has to refuse them on its own rather
# than relying on the writer's checks.
#   desktop/packaging/update_feed_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
desktop=$(dirname "$here")
fixtures="$desktop/core/tests/fixtures/update-feed"
keys="$fixtures/test-keys.json"
pem="$fixtures/test-key-linux-feed-test.pem"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "update_feed_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1" reason="$2" err="$work/stderr"
  shift 2
  if "$@" > /dev/null 2> "$err"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$err" || fail "$what was refused for another reason: $(cat "$err")"
  echo "  refused: $what"
}

feed() {
  "$here/update_feed.py" "$@"
}

hex_to_bytes() {
  python3 -I -c 'import sys; sys.stdout.buffer.write(bytes.fromhex(sys.argv[1]))' "$1"
}

bytes_to_hex() {
  od -An -v -tx1 | tr -d ' \n'
}

# <seed hex>: the ed25519 key with that seed, as PKCS#8 PEM.
pem_of_seed() {
  python3 -I -c 'import base64, sys
der = bytes.fromhex("302e020100300506032b657004220420" + sys.argv[1])
sys.stdout.write("-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(der).decode() + "\n-----END PRIVATE KEY-----\n")' "$1"
}

# <out> <payload file>: an envelope over <payload file>'s bytes, signed by the test key as they are.
raw_envelope() {
  local out="$1" payload="$2"
  openssl pkeyutl -sign -rawin -inkey "$pem" -in "$payload" -out "$work/raw.sig"
  jq -n --arg payload "$(base64 -w 0 < "$payload")" --arg signature "$(base64 -w 0 < "$work/raw.sig")" \
    '{format: 1, key_id: "linux-feed-test", payload: $payload, signature: $signature}' > "$out"
}

# <out> <jq filter>: the fixture's payload changed by <filter>, signed by the test key.
signed_variant() {
  jq -c ".payload | $2" "$fixtures/expected-decode.json" > "$work/variant.payload"
  raw_envelope "$1" "$work/variant.payload"
}

# <dir> <version>: the four packages of <version>, each a line of text.
fake_packages() {
  local dir="$1" version="$2" name
  mkdir -p "$dir"
  for name in "fermix-desktop_${version}_amd64.deb" "fermix-desktop-${version}-1.x86_64.rpm" \
    "fermix-desktop_${version}_arm64.deb" "fermix-desktop-${version}-1.aarch64.rpm"; do
    printf 'package %s\n' "$name" > "$dir/$name"
  done
}

echo "update_feed_test: the shared fixture is what the writer writes"
mkdir "$work/fixture"
"$here/update_feed_fixture.sh" "$work/fixture" > /dev/null
generated=(appcast.json expected-decode.json test-key-linux-feed-test.pem test-key-linux-feed-test.seed test-keys.json)
for name in "${generated[@]}"; do
  cmp -s "$work/fixture/$name" "$fixtures/$name" || fail "the committed $name is not what update_feed_fixture.sh writes"
done
[ "$(cd "$fixtures" && find . -type f -printf '%f\n' | LC_ALL=C sort | tr '\n' ' ')" = "SOURCE.md ${generated[*]} " ] ||
  fail "the fixture directory holds other files: $(ls "$fixtures")"
feed verify --feed "$fixtures/appcast.json" --keys "$keys" --decoded "$work/decoded.json" > /dev/null
cmp -s "$work/decoded.json" "$fixtures/expected-decode.json" || fail "verify decodes the fixture differently"
echo "  ok: every file, byte for byte, and verify decodes it as expected-decode.json"

echo "update_feed_test: OpenSSL signs RFC 8032's Ed25519, from a key on standard input"
# RFC 8032 section 7.1, tests 2 and 3: <seed> <message hex> <signature>. Test 1's empty message is
# left out: OpenSSL 3.0.2 cannot sign zero bytes, and a payload is never empty.
while read -r seed message signature; do
  hex_to_bytes "$message" > "$work/vector.message"
  pem_of_seed "$seed" | openssl pkeyutl -sign -rawin -inkey /dev/stdin -in "$work/vector.message" > "$work/vector.sig"
  [ "$(bytes_to_hex < "$work/vector.sig")" = "$signature" ] || fail "OpenSSL signs the RFC 8032 message $message otherwise"
  echo "  ok: the message '$message'"
done << 'EOF'
4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb 72 92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00
c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7 af82 6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a
EOF

echo "update_feed_test: the keys the app trusts"
production="$here/update-keys.json"
[ "$(jq -c '[.keys[] | [.key_id, .role, .public_key]]' "$production")" = \
  '[["linux-feed-1","current","E51Uv2uksKNCEBeij/hJZ7dRgiUNFsz5pmzyX8iXwfY="],["linux-feed-2","standby","WLIAl6ZJLQcRV5RV87cIJ3L/ALzLZrJI0jEemRMJG2A="]]' ] ||
  fail "update-keys.json is not linux-feed-1, current, and linux-feed-2, standby: $(jq -c . "$production")"
test_public="$(jq -r '.keys[0].public_key' "$keys")"
if grep -qF -e linux-feed-test -e "$test_public" "$production"; then
  fail "update-keys.json lists the test key"
fi
echo "  ok: linux-feed-1 current, linux-feed-2 standby, and not the test key by id or public key"
base64 -d <<< "$(jq -r .payload "$fixtures/appcast.json")" > "$work/fixture.payload"
expect_refusal "the test key signing as the current production key" \
  "the key on standard input is not linux-feed-1's" \
  feed sign --payload "$work/fixture.payload" --keys "$production" --out "$work/x.json" < "$pem"
expect_refusal "a feed the test key signed, against the production keys" \
  "is signed by linux-feed-test, which the keys file does not list" \
  feed verify --feed "$fixtures/appcast.json" --keys "$production"
[ ! -e "$work/x.json" ] || fail "a refused signature left a feed behind"

# The rest of the subcommands, with a pin engine_pin.py accepts and a release's notes.
pin="$work/PIN.json"
cp "$desktop/engine/PIN.json" "$pin"
engine="$(jq -r .engine_version "$pin")"
fake_packages "$work/packages" 0.6.0
# shellcheck disable=SC2016 # a `code` span, not a command
printf -- '- Fermix is faster.\n- Run `fermix status` to see it.\n' > "$work/notes.md"
item() {
  feed item --packages "$work/packages" --version 0.6.0 --pin "$pin" --notes "${NOTES:-$work/notes.md}" \
    --base-url "${BASE_URL:-https://downloads.example.com/v0.6.0/}" --published-at 2026-12-01T10:00:00Z \
    --out "$work/item.json"
}

echo "update_feed_test: item"
item > /dev/null
# shellcheck disable=SC2016 # a `code` span, not a command
[ "$(jq -c '[.version, .published_at, .critical, .minimum_installed_version, .notes]' "$work/item.json")" = \
  '["0.6.0","2026-12-01T10:00:00Z",false,null,["Fermix is faster.","Run `fermix status` to see it."]]' ] ||
  fail "item wrote another release: $(jq -c . "$work/item.json")"
[ "$(jq -c .engine "$work/item.json")" = "$(jq -c '{version: .engine_version, tag, source_commit}' "$pin")" ] ||
  fail "item's engine is not the pin's: $(jq -c .engine "$work/item.json")"
rpm="fermix-desktop-0.6.0-1.aarch64.rpm"
[ "$(jq -c .packages.arm64.rpm "$work/item.json")" = "$(jq -nc --arg sha "$(sha256sum < "$work/packages/$rpm" | cut -c1-64)" \
  --arg file "$rpm" --argjson size "$(wc -c < "$work/packages/$rpm")" \
  '{file: $file, url: "https://downloads.example.com/v0.6.0/\($file)", size: $size, sha256: $sha}')" ] ||
  fail "item measured the arm64 rpm otherwise: $(jq -c .packages.arm64.rpm "$work/item.json")"
echo "  ok: engine $engine from the pin, the notes in order, each package measured and hashed"
printf -- '---\ncritical: true\nminimum_installed_version: 0.5.2\n---\n- Fermix is safer.\n' > "$work/front.md"
NOTES="$work/front.md" item > /dev/null
[ "$(jq -c '[.critical, .minimum_installed_version]' "$work/item.json")" = '[true,"0.5.2"]' ] ||
  fail "item did not take critical and the minimum from the front matter"
printf -- '---\nminimum_installed_version: 0.6.0\n---\n- Fermix is the same.\n' > "$work/front.md"
NOTES="$work/front.md" item > /dev/null
echo "  ok: critical and minimum_installed_version from the front matter, a minimum of itself too"
for notes in "$here"/notes/*.md; do
  version="$(basename "$notes" .md)"
  fake_packages "$work/released/$version" "$version"
  feed item --packages "$work/released/$version" --version "$version" --pin "$desktop/engine/PIN.json" \
    --notes "$notes" --base-url "https://github.com/tezra-io/fermix-linux/releases/download/v$version" \
    --published-at 2026-12-01T10:00:00Z --out "$work/released/$version.json" > /dev/null ||
    fail "the notes of $version are not ones a release can carry"
  echo "  ok: notes/$version.md"
done

echo "update_feed_test: what item refuses"
expect_refusal "a version with a leading zero" "is \"0.06.0\", which is not a release version X.Y.Z" \
  feed item --packages "$work/packages" --version 0.06.0 --pin "$pin" --notes "$work/notes.md" \
  --base-url https://example.com/x --published-at 2026-12-01T10:00:00Z --out "$work/x.json"
expect_refusal "a development version" "which is not a release version X.Y.Z" \
  feed item --packages "$work/packages" --version 0.6.0+dev.20261201000000.0123456789ab --pin "$pin" \
  --notes "$work/notes.md" --base-url https://example.com/x --published-at 2026-12-01T10:00:00Z --out "$work/x.json"
rm "$work/packages/fermix-desktop_0.6.0_arm64.deb"
expect_refusal "a missing package" "there is no package fermix-desktop_0.6.0_arm64.deb in" item
fake_packages "$work/packages" 0.6.0
jq '.tag = "v0.0.1"' "$pin" > "$work/bad-pin.json"
expect_refusal "a pin engine_pin.py refuses" "is not one a release can carry" \
  feed item --packages "$work/packages" --version 0.6.0 --pin "$work/bad-pin.json" --notes "$work/notes.md" \
  --base-url https://example.com/x --published-at 2026-12-01T10:00:00Z --out "$work/x.json"
expect_refusal "a time with a fraction" "which is not a UTC time" \
  feed item --packages "$work/packages" --version 0.6.0 --pin "$pin" --notes "$work/notes.md" \
  --base-url https://example.com/x --published-at 2026-12-01T10:00:00.5Z --out "$work/x.json"
expect_refusal "a time that is not one" "which is not a real time" \
  feed item --packages "$work/packages" --version 0.6.0 --pin "$pin" --notes "$work/notes.md" \
  --base-url https://example.com/x --published-at 2026-02-30T10:00:00Z --out "$work/x.json"
# <what> <reason> <base url>
for case in "http|which is not https to a host|http://example.com/v0.6.0" \
  "a user|with no user or port|https://user@example.com/v0.6.0" \
  "a port|with no user or port|https://example.com:8443/v0.6.0" \
  "a query|which carries a character a url escapes|https://example.com/v0.6.0?x=" \
  "a space|which carries a character a url escapes|https://example.com/v 0.6.0" \
  "no host|which is not https to a host|https:///v0.6.0"; do
  IFS='|' read -r what reason url <<< "$case"
  BASE_URL="$url" expect_refusal "a base url with $what" "$reason" item
done
# <what> <reason> <notes, printf format>
while IFS='|' read -r what reason notes; do
  # shellcheck disable=SC2059 # the case's notes are a format, for their escapes
  printf -- "$notes" > "$work/bad.md"
  NOTES="$work/bad.md" expect_refusal "notes with $what" "$reason" item
done << 'EOF'
no newline at the end|do not end every line with one newline|- Fermix is faster.
a line that is not a bullet|line 2 of the notes|- Fermix is faster.\nFermix is safer.\n
a blank line|line 2 of the notes|- Fermix is faster.\n\n- Fermix is safer.\n
a carriage return|do not end every line with one newline|- Fermix is faster.\r\n
no notes|line 1 of the notes|\n
nine notes|no list of 1 to 8 notes|- A.\n- B.\n- C.\n- D.\n- E.\n- F.\n- G.\n- H.\n- I.\n
a leading space|begins or ends with a space|-  Fermix is faster.\n
emphasis|carries "*" outside a `code` span|- Fermix is *faster*.\n
a link|carries "[" outside a `code` span|- See [the notes](https://fermix.ai).\n
an underscore|carries "_" outside a `code` span|- Set FERMIX_HOME first.\n
an open code span|opens a `code` span it does not close|- Run `fermix status.\n
a tab|hidden or control character U+0009|- Fermix\tis faster.\n
no full stop|does not end a sentence|- Fermix is faster\n
an unknown key|and minimum_installed_version: X.Y.Z, and nothing else|---\nchannel: beta\n---\n- Fermix is faster.\n
an unclosed front matter|opens with --- and never closes|---\ncritical: true\n- Fermix is faster.\n
critical as yes|and minimum_installed_version: X.Y.Z, and nothing else|---\ncritical: yes\n---\n- Fermix is faster.\n
a key twice|not one new key: value|---\ncritical: true\ncritical: false\n---\n- Fermix is faster.\n
a minimum newer than itself|asks for 0.7.0 installed first, which is newer than itself|---\nminimum_installed_version: 0.7.0\n---\n- Fermix is faster.\n
a minimum that is no version|is "0.5", which is not a release version|---\nminimum_installed_version: 0.5\n---\n- Fermix is faster.\n
EOF
python3 -I -c 'import sys; open(sys.argv[1], "w", encoding="utf-8").write("- " + "é" * 299 + ".\n")' "$work/long.md"
NOTES="$work/long.md" item > /dev/null
python3 -I -c 'import sys; open(sys.argv[1], "w", encoding="utf-8").write("- " + "é" * 300 + ".\n")' "$work/long.md"
NOTES="$work/long.md" expect_refusal "a note of 301 characters" "is not a sentence of 1 to 300 characters" item
printf -- '- Fermix is \xe2\x80\xaeretsaf.\n' > "$work/bidi.md"
NOTES="$work/bidi.md" expect_refusal "a right-to-left override" "hidden or control character U+202E" item
echo "  ok: a note of 300 characters, counted as characters rather than bytes, is accepted"
[ ! -e "$work/x.json" ] || fail "a refused item left a file behind"

echo "update_feed_test: merge"
item > /dev/null
feed merge --item "$work/item.json" --keys "$keys" --generated-at 2026-12-01T10:05:00Z --out "$work/first.payload" > /dev/null
[ "$(jq -c '[.format, .product, .generated_at, [.releases[].version]]' "$work/first.payload")" = \
  '[1,"fermix-desktop","2026-12-01T10:05:00Z",["0.6.0"]]' ] || fail "the first feed is not the item alone"
merge_onto_fixture() {
  feed merge --item "$work/item.json" --previous "${PREVIOUS:-$fixtures/appcast.json}" --keys "$keys" \
    --generated-at "${GENERATED_AT:-2026-12-01T10:05:00Z}" --out "$work/merged.payload"
}
merge_onto_fixture > /dev/null
[ "$(jq -c '[.generated_at, [.releases[].version]]' "$work/merged.payload")" = \
  '["2026-12-01T10:05:00Z",["0.6.0","0.5.0","0.4.0","0.3.0"]]' ] || fail "the merged feed is not the item over the fixture's"
[ "$(jq -c '.releases[1:]' "$work/merged.payload")" = "$(jq -c .payload.releases "$fixtures/expected-decode.json")" ] ||
  fail "the merge changed a release the fixture already carried"
echo "  ok: the first feed, and 0.6.0 in front of the fixture's three, which it carries unchanged"
GENERATED_AT=2026-11-20T18:05:00Z expect_refusal "a feed no later than the one before" \
  "2026-11-20T18:05:00Z is not later than the previous feed" merge_onto_fixture
for version in 0.5.0 0.4.9; do
  fake_packages "$work/older" "$version"
  feed item --packages "$work/older" --version "$version" --pin "$pin" --notes "$work/notes.md" \
    --base-url https://example.com/x --published-at 2026-12-01T10:00:00Z --out "$work/older.json" > /dev/null
  expect_refusal "the version $version over a feed whose newest is 0.5.0" \
    "$version is not higher than 0.5.0, the newest release the feed already carries" \
    feed merge --item "$work/older.json" --previous "$fixtures/appcast.json" --keys "$keys" \
    --generated-at 2026-12-01T10:05:00Z --out "$work/x.json"
done
jq '.payload |= (.[:-8] + "AAAAAAA=")' "$fixtures/appcast.json" > "$work/tampered.json"
PREVIOUS="$work/tampered.json" expect_refusal "a previous feed whose payload was changed" \
  "does not verify against linux-feed-test" merge_onto_fixture
expect_refusal "a previous feed signed by a key the file does not list" \
  "is signed by linux-feed-test, which the keys file does not list" \
  feed merge --item "$work/item.json" --previous "$fixtures/appcast.json" --keys "$production" \
  --generated-at 2026-12-01T10:05:00Z --out "$work/x.json"
expect_refusal "an item that breaks a rule" "has the size 0, not 1 to 536870912 bytes" \
  feed merge --item <(jq '.packages.amd64.deb.size = 0' "$work/item.json") --keys "$keys" \
  --generated-at 2026-12-01T10:05:00Z --out "$work/x.json"
# 200 releases, 0.1.200 down to 0.1.1, each the fixture's 0.3.0 renamed.
python3 -I -c 'import json, sys
release = json.load(open(sys.argv[1]))["payload"]["releases"][2]
text = json.dumps(release)
releases = [json.loads(text.replace("0.3.0", f"0.1.{n}")) for n in range(200, 0, -1)]
payload = {"format": 1, "product": "fermix-desktop", "generated_at": "2026-12-01T09:00:00Z", "releases": releases}
open(sys.argv[2], "w").write(json.dumps(payload, separators=(",", ":")))' "$fixtures/expected-decode.json" "$work/full.payload"
feed sign --payload "$work/full.payload" --keys "$keys" --out "$work/full.json" < "$pem" > /dev/null
PREVIOUS="$work/full.json" merge_onto_fixture > /dev/null
[ "$(jq -c '[(.releases | length), .releases[0].version, .releases[1].version, .releases[-1].version]' "$work/merged.payload")" = \
  '[200,"0.6.0","0.1.200","0.1.2"]' ] || fail "the merge did not keep the newest 200"
echo "  ok: a full feed keeps the newest 200: 0.1.1 is dropped for 0.6.0"
[ ! -e "$work/x.json" ] || fail "a refused merge left a file behind"

echo "update_feed_test: sign"
feed sign --payload "$work/merged.payload" --keys "$keys" --out "$work/signed.json" < "$pem" > /dev/null
feed verify --feed "$work/signed.json" --keys "$keys" > /dev/null
cmp -s <(base64 -d <<< "$(jq -r .payload "$work/signed.json")") "$work/merged.payload" ||
  fail "the signed feed does not carry the payload's bytes as they are"
[ "$(jq -c 'keys_unsorted' "$work/signed.json")" = '["format","key_id","payload","signature"]' ] ||
  fail "the envelope has other fields: $(jq -c keys_unsorted "$work/signed.json")"
echo "  ok: the payload's bytes as they were, signed by the current key, verified"
other="$(pem_of_seed 4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb)"
expect_refusal "another ed25519 key" "the key on standard input is not linux-feed-test's" \
  feed sign --payload "$work/merged.payload" --keys "$keys" --out "$work/x.json" <<< "$other"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$work/ec.pem" 2> /dev/null
expect_refusal "an ECDSA key" "is not an ed25519 key" \
  feed sign --payload "$work/merged.payload" --keys "$keys" --out "$work/x.json" < "$work/ec.pem"
expect_refusal "no key" "openssl could not sign the payload" \
  feed sign --payload "$work/merged.payload" --keys "$keys" --out "$work/x.json" < /dev/null
jq -c '.releases |= reverse' "$work/merged.payload" > "$work/reversed.payload"
expect_refusal "a payload out of order" "are not newest first" \
  feed sign --payload "$work/reversed.payload" --keys "$keys" --out "$work/x.json" < "$pem"
head -c 1048577 /dev/zero > "$work/huge.payload"
expect_refusal "a payload over 1 MiB" "is larger than 1048576 bytes" \
  feed sign --payload "$work/huge.payload" --keys "$keys" --out "$work/x.json" < "$pem"
jq '.keys += [{key_id: "linux-feed-test-2", role: "standby", public_key: .keys[0].public_key}]' "$keys" > "$work/keys.json"
expect_refusal "a keys file that lists one public key twice" "lists linux-feed-test-2 or its public key twice" \
  feed sign --payload "$work/merged.payload" --keys "$work/keys.json" --out "$work/x.json" < "$pem"
other_public="$(hex_to_bytes 3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c | base64 -w 0)"
jq --arg public "$other_public" '.keys += [{key_id: "linux-feed-test-2", role: "current", public_key: $public}]' \
  "$keys" > "$work/keys.json"
expect_refusal "a keys file with two current keys" "not one current and one standby" \
  feed sign --payload "$work/merged.payload" --keys "$work/keys.json" --out "$work/x.json" < "$pem"
jq '.keys[0].public_key = "AAAA"' "$keys" > "$work/keys.json"
expect_refusal "a keys file with a short public key" "the public key of linux-feed-test is not 32 bytes" \
  feed sign --payload "$work/merged.payload" --keys "$work/keys.json" --out "$work/x.json" < "$pem"
[ ! -e "$work/x.json" ] || fail "a refused signature left a feed behind"

echo "update_feed_test: what verify refuses in the envelope"
# <what> <reason> <jq filter over the fixture's envelope>
while IFS='|' read -r what reason filter; do
  jq "$filter" "$fixtures/appcast.json" > "$work/envelope.json"
  expect_refusal "an envelope with $what" "$reason" feed verify --feed "$work/envelope.json" --keys "$keys"
done << 'EOF'
a field it does not know|not ["format", "key_id", "payload", "signature"]|.channel = "beta"
no signature|not ["format", "key_id", "payload", "signature"]|del(.signature)
format 2|is format 2, and this is format 1|.format = 2
format true|is format true, and this is format 1|.format = true
a key id out of form|has the key_id "Linux-Feed", not [a-z0-9-]{1,64}|.key_id = "Linux-Feed"
a key it does not know|is signed by linux-feed-9, which the keys file does not list|.key_id = "linux-feed-9"
a payload that is not base64|is not standard base64 with padding|.payload = "not base64!"
a payload without its padding|is not standard base64 with padding|.payload |= rtrimstr("=")
a payload in another spelling|is not base64 in its canonical form|.payload |= (.[:-3] + "R==")
a short signature|is not 64 bytes|.signature = "AAAA"
a signature of other bytes|does not verify against linux-feed-test|.signature |= ("AAAA" + .[4:])
a changed payload|does not verify against linux-feed-test|.payload |= (.[:-8] + "AAAAAAA=")
EOF
sed 's/"format": 1,/"format": 1, "format": 1,/' "$fixtures/appcast.json" > "$work/envelope.json"
expect_refusal "an envelope with a key twice" "the key \"format\" appears twice in one object" \
  feed verify --feed "$work/envelope.json" --keys "$keys"
{ cat "$fixtures/appcast.json"; head -c 1048576 /dev/zero | tr '\0' ' '; } > "$work/envelope.json"
expect_refusal "an envelope over 1 MiB" "is larger than 1048576 bytes" feed verify --feed "$work/envelope.json" --keys "$keys"

echo "update_feed_test: what verify refuses in a payload the key signed"
# <what> <reason> <jq filter over the fixture's payload>
while IFS='|' read -r what reason filter; do
  signed_variant "$work/envelope.json" "$filter"
  expect_refusal "a payload with $what" "$reason" feed verify --feed "$work/envelope.json" --keys "$keys"
done << 'EOF'
another product|is for "fermix", not fermix-desktop|.product = "fermix"
format 2|is format 2, and this is format 1|.format = 2
no releases|does not list 1 to 200 releases|.releases = []
releases oldest first|are not newest first|.releases |= reverse
a release twice|are not newest first|.releases |= [.[0], .[0], .[1]]
a version with a build part|is "0.5.0+1", which is not a release version|.releases[0].version = "0.5.0+1"
a generated_at with an offset|which is not a UTC time|.generated_at = "2026-11-20T18:05:00+00:00"
critical as a string|says critical is "true", not true or false|.releases[1].critical = "true"
no minimum_installed_version|has no minimum_installed_version|del(.releases[0].minimum_installed_version)
a minimum newer than its release|asks for 0.6.0 installed first|.releases[0].minimum_installed_version = "0.6.0"
an engine tag that is not its version's|names the tag "v0.18.1"|.releases[0].engine.tag = "v0.18.1"
a short source commit|not 40 lowercase hex|.releases[0].engine.source_commit = "af3419ac"
no notes|has no list of 1 to 8 notes|.releases[0].notes = []
a note with markup|carries "*" outside a `code` span|.releases[0].notes[0] = "Fermix is *fast*."
a package missing|the arm64 packages of|del(.releases[0].packages.arm64)
a format missing|the amd64 rpm of release 1|del(.releases[0].packages.amd64.rpm)
another file name|names the file "fermix-desktop_0.5.0_amd64.deb.zip"|.releases[0].packages.amd64.deb.file = "fermix-desktop_0.5.0_amd64.deb.zip"
a url for another file|which does not end in /fermix-desktop_0.5.0_amd64.deb|.releases[0].packages.amd64.deb.url = "https://example.com/fermix-desktop_0.4.0_amd64.deb"
an http url|which is not https to a host|.releases[0].packages.amd64.deb.url |= sub("^https"; "http")
a size of 0|has the size 0|.releases[0].packages.amd64.deb.size = 0
a size over 512 MiB|has the size 536870913|.releases[0].packages.amd64.deb.size = 536870913
a size as text|has the size "44"|.releases[0].packages.amd64.deb.size = "44"
a size as true|has the size true|.releases[0].packages.amd64.deb.size = true
an uppercase sha256|not 64 lowercase hex|.releases[0].packages.amd64.deb.sha256 |= ascii_upcase
EOF
printf '\xef\xbb\xbf' > "$work/bom.payload"
jq -c .payload "$fixtures/expected-decode.json" >> "$work/bom.payload"
raw_envelope "$work/envelope.json" "$work/bom.payload"
expect_refusal "a payload behind a byte-order mark" "is not valid JSON" feed verify --feed "$work/envelope.json" --keys "$keys"
printf '{"format":1,"format":1}' > "$work/twice.payload"
raw_envelope "$work/envelope.json" "$work/twice.payload"
expect_refusal "a payload with a key twice" "the key \"format\" appears twice in one object" \
  feed verify --feed "$work/envelope.json" --keys "$keys"
signed_variant "$work/envelope.json" '.mirror = "https://example.com" | .releases[0].channel = "beta"'
feed verify --feed "$work/envelope.json" --keys "$keys" > /dev/null ||
  fail "verify refused a payload with fields it does not know, which the format allows"
echo "  ok: a payload with fields it does not know, at the top and in a release, is accepted"

echo "update_feed_test: usage"
expect_refusal "no subcommand" "usage" feed
expect_refusal "an unknown subcommand" "usage" feed publish
expect_refusal "a flag missing" "usage" feed verify --feed "$fixtures/appcast.json"
expect_refusal "the key as an argument" "usage" feed sign --payload "$work/merged.payload" --keys "$keys" \
  --out "$work/x.json" --key "$pem"

echo "update_feed_test: ok"
