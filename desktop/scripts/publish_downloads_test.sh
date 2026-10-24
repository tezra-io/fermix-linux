#!/usr/bin/env bash
# Offline tests for publish_downloads.sh. Its refusals guard files that can never be taken back:
# once a version is on download.fermix.ai, the bucket's lock rule keeps it, and the update feed and
# the site point people at it. So each refusal is fired here on purpose: a folder that is not a
# release's, a key that already holds other bytes, a host that serves something other than what was
# uploaded, a failed upload, and a missing credential.
#
# The script is handed an `aws` and a `curl` that stand in for the bucket and the public host,
# backed by one folder: the stand-in `aws` writes a key only when it holds nothing and answers
# PreconditionFailed otherwise, as R2 does for --if-none-match '*', and the stand-in `curl` serves
# what that folder holds. Every run gets a scrubbed environment, so no credential of the host's
# reaches the script. Nothing here reaches the network.
#   desktop/scripts/publish_downloads_test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
bin="$work/bin"
bucket="$work/bucket"
control="$work/control"
aws_log="$work/aws.log"
curl_log="$work/curl.log"
PREFIX=linux/v1.2.3

fail() {
  echo "publish_downloads_test: $*" >&2
  exit 1
}

mkdir -p "$bin" "$bucket" "$control"

# The bucket. put-object logs its arguments and the environment it was given, then writes the body
# under the key; asked to write once, it refuses a key that holds something.
cat > "$bin/aws" << STUB
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$*" >> "$aws_log"
printf 'env %s %s %s %s %s\n' "\$AWS_ACCESS_KEY_ID" "\$AWS_SECRET_ACCESS_KEY" "\$AWS_DEFAULT_REGION" \
  "\$AWS_REQUEST_CHECKSUM_CALCULATION" "\$AWS_RESPONSE_CHECKSUM_VALIDATION" >> "$aws_log"
if [ -e "$control/denied" ]; then
  echo "An error occurred (AccessDenied) when calling the PutObject operation: Access Denied" >&2
  exit 254
fi
key="" body="" once=""
while [ "\$#" -gt 0 ]; do
  case "\$1" in
    --key) key="\$2"; shift 2 ;;
    --body) body="\$2"; shift 2 ;;
    --if-none-match) once="\$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [ "\$once" = "*" ] && [ -e "$bucket/\$key" ]; then
  echo "An error occurred (PreconditionFailed) when calling the PutObject operation: At least one of the pre-conditions you specified did not hold" >&2
  exit 254
fi
mkdir -p "\$(dirname "$bucket/\$key")"
cp "\$body" "$bucket/\$key"
echo '{"ETag": "\\"stand-in\\""}'
STUB

# The public host: the last argument is the url, served from the bucket folder, altered on the way
# or not served at all when a case asks for it.
cat > "$bin/curl" << STUB
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$*" >> "$curl_log"
[ ! -e "$control/unserved" ] || exit 22
for url in "\$@"; do :; done
path="$bucket/\${url#https://download.fermix.ai/}"
[ "\$url" != "\${url#https://download.fermix.ai/}" ] && [ -f "\$path" ] || exit 22
cat "\$path"
[ ! -e "$control/altered" ] || printf 'altered'
STUB
chmod 0755 "$bin/aws" "$bin/curl"

CREDENTIALS=(R2_ACCESS_KEY_ID=key-id R2_SECRET_ACCESS_KEY=secret R2_ENDPOINT=https://account.r2.example.invalid)

# Runs the script with the stand-ins first on PATH and a throwaway HOME.
publish() {
  env -i PATH="$bin:$PATH" HOME="$work" "$@"
}

expect_pass() {
  local what="$1"
  shift
  "$@" > "$work/out" 2>&1 || fail "$what was refused: $(tail -3 "$work/out")"
  echo "  passed: $what"
}

expect_refusal() {
  local what="$1" reason="$2"
  shift 2
  if "$@" > "$work/out" 2>&1; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$work/out" || fail "$what was refused for another reason: $(tail -3 "$work/out")"
  echo "  refused: $what"
}

# Clears the bucket, the logs and the switches between cases.
fresh() {
  rm -rf "$bucket" "$control" "$aws_log" "$curl_log"
  mkdir -p "$bucket" "$control"
}

# One release's files, as the release has them: each package and the sources archive with its
# checksum, signature and certificate.
files="$work/release"
mkdir -p "$files"
RELEASE=()
for name in fermix-desktop_1.2.3_amd64.deb fermix-desktop-1.2.3-1.x86_64.rpm \
  fermix_desktop_runtime_sources_1.2.3.tar.gz; do
  printf '%s bytes' "$name" > "$files/$name"
  printf 'abc  %s\n' "$name" > "$files/$name.sha256"
  printf 'signature' > "$files/$name.sig"
  printf -- '-----BEGIN CERTIFICATE-----\n' > "$files/$name.pem"
  RELEASE+=("$files/$name" "$files/$name.sha256" "$files/$name.sig" "$files/$name.pem")
done

echo "publish_downloads_test: what is refused before anything is uploaded"
expect_refusal "no arguments" "usage:" publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh"
expect_refusal "a folder and no file" "usage:" publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX"
for folder in "$PREFIX/" linux/1.2.3 macos/v1.2.3 linux/v1.2 "linux/v1.2.3/x" ""; do
  expect_refusal "the folder '$folder'" "is not a release's folder, linux/vX.Y.Z" \
    publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$folder" "${RELEASE[@]}"
done
for missing in R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_ENDPOINT; do
  given=()
  for credential in "${CREDENTIALS[@]}"; do
    [ "${credential%%=*}" = "$missing" ] || given+=("$credential")
  done
  expect_refusal "no $missing" "$missing is required" \
    publish "${given[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}"
done
fresh
expect_refusal "a file that is not there" "no file at $files/fermix-desktop_1.2.3_arm64.deb" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}" \
  "$files/fermix-desktop_1.2.3_arm64.deb"
[ ! -e "$aws_log" ] || fail "a missing file still let something upload"
printf '{}' > "$files/appcast.json"
expect_refusal "a file with no content type" "no content type for $files/appcast.json" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}" "$files/appcast.json"
[ ! -e "$aws_log" ] || fail "a file with no content type still let something upload"

echo "publish_downloads_test: a release's files"
fresh
expect_pass "every file is uploaded and read back" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}"
for file in "${RELEASE[@]}"; do
  cmp -s "$file" "$bucket/$PREFIX/$(basename "$file")" || fail "$(basename "$file") is not in the bucket under $PREFIX"
  grep -qxF "publish_downloads: published https://download.fermix.ai/$PREFIX/$(basename "$file")" "$work/out" ||
    fail "$(basename "$file") was not reported as published: $(cat "$work/out")"
done
grep -q -- "--key $PREFIX/fermix-desktop_1.2.3_amd64.deb .*--content-type application/vnd.debian.binary-package --cache-control public, max-age=31536000, immutable --if-none-match \* --endpoint-url https://account.r2.example.invalid --bucket fermix-downloads" "$aws_log" ||
  fail "the deb was not written once, with its type and cache lifetime, to fermix-downloads: $(head -1 "$aws_log")"
grep -q -- "--key $PREFIX/fermix-desktop-1.2.3-1.x86_64.rpm .*--content-type application/x-rpm " "$aws_log" || fail "the rpm's type"
grep -q -- "--key $PREFIX/fermix_desktop_runtime_sources_1.2.3.tar.gz .*--content-type application/gzip " "$aws_log" ||
  fail "the sources archive's type"
grep -q -- "--key $PREFIX/fermix-desktop_1.2.3_amd64.deb.pem .*--content-type application/x-pem-file " "$aws_log" || fail "the certificate's type"
grep -q -- "--key $PREFIX/fermix-desktop_1.2.3_amd64.deb.sig .*--content-type text/plain " "$aws_log" || fail "the signature's type"
grep -q -- "--key $PREFIX/fermix-desktop_1.2.3_amd64.deb.sha256 .*--content-type text/plain " "$aws_log" || fail "the checksum's type"
grep -qxF "env key-id secret auto when_required when_required" "$aws_log" ||
  fail "the R2 credentials, region and checksum settings did not reach aws: $(sed -n 2p "$aws_log")"
grep -qF -- "--retry 10 --retry-all-errors --retry-delay 20 https://download.fermix.ai/$PREFIX/fermix-desktop_1.2.3_amd64.deb" "$curl_log" ||
  fail "the read-back is not a bounded retry of the public url: $(head -1 "$curl_log")"
echo "  passed: with their types, the year-long cache lifetime, the R2 credentials and a bounded read-back"
expect_pass "a re-run of the same tag accepts the same bytes" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}"
grep -qxF "publish_downloads: already published https://download.fermix.ai/$PREFIX/fermix-desktop_1.2.3_amd64.deb" "$work/out" ||
  fail "the re-run did not report the deb as already published: $(cat "$work/out")"

echo "publish_downloads_test: what is refused while publishing"
fresh
mkdir -p "$bucket/$PREFIX"
printf 'other bytes' > "$bucket/$PREFIX/fermix-desktop_1.2.3_amd64.deb"
expect_refusal "a key that already holds other bytes" "already holds other bytes" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}"
[ "$(cat "$bucket/$PREFIX/fermix-desktop_1.2.3_amd64.deb")" = "other bytes" ] || fail "the published file was replaced"
fresh
touch "$control/altered"
expect_refusal "a host that serves other bytes than were uploaded" "serves sha256" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}"
fresh
touch "$control/denied"
expect_refusal "an upload the bucket refuses" \
  "could not upload $PREFIX/fermix-desktop_1.2.3_amd64.deb: An error occurred (AccessDenied)" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}"
fresh
touch "$control/unserved"
expect_refusal "a file the host does not serve" \
  "could not read https://download.fermix.ai/$PREFIX/fermix-desktop_1.2.3_amd64.deb back" \
  publish "${CREDENTIALS[@]}" "$here/publish_downloads.sh" "$PREFIX" "${RELEASE[@]}"

echo "publish_downloads_test: ok"
