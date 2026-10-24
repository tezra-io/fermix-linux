#!/usr/bin/env bash
#
# Publishes a release's files to download.fermix.ai, the host every public download comes from:
# the update feed's package urls and the site's links point there, so they keep working when this
# repository is private.
#
#   publish_downloads.sh linux/vX.Y.Z <file>...
#
# Each file goes to the R2 bucket behind the host as linux/vX.Y.Z/<file name>, and is written once:
# the upload only lands on a key that holds nothing, and the bucket's lock rule keeps a published
# version's files for good. A key that already holds the same bytes is a re-run of the same tag and
# passes; a key that holds other bytes is refused, because a published file never changes under the
# people who already downloaded it. Every file is then read back through the public host and its
# sha256 compared, so only bytes a user would actually receive count as published. The macOS app's
# scripts/publish_downloads.sh does the same for its files.
#
# The credentials are the release-linux environment's R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY and
# R2_ENDPOINT (the account's S3 endpoint).
set -euo pipefail
shopt -s inherit_errexit

BUCKET=fermix-downloads
PUBLIC_BASE=https://download.fermix.ai
# A published file never changes, so any cache may keep it for a year.
CACHE_CONTROL="public, max-age=31536000, immutable"

fail() {
  echo "publish_downloads: $*" >&2
  exit 1
}

# The types v0.2.0's files were uploaded with, so every version is served the same way.
content_type() {
  case "$1" in
    *.deb) echo application/vnd.debian.binary-package ;;
    *.rpm) echo application/x-rpm ;;
    *.tar.gz) echo application/gzip ;;
    *.pem) echo application/x-pem-file ;;
    *.sha256 | *.sig) echo text/plain ;;
    *) return 1 ;;
  esac
}

# R2 speaks the S3 API in the region "auto". The CLI adds checksums only where an operation
# requires one, the settings the bucket was proven with.
s3() {
  AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY" \
    AWS_DEFAULT_REGION=auto AWS_REQUEST_CHECKSUM_CALCULATION=when_required \
    AWS_RESPONSE_CHECKSUM_VALIDATION=when_required \
    aws s3api "$@" --endpoint-url "$R2_ENDPOINT" --bucket "$BUCKET"
}

sha256_of() {
  sha256sum "$1" | awk '{print $1}'
}

# Ten tries 20 seconds apart cover the moment between the upload and the host serving it, and a
# 404 the host may have cached for the url before the upload.
public_sha256() {
  curl -fsSL --retry 10 --retry-all-errors --retry-delay 20 "$PUBLIC_BASE/$1" | sha256sum | awk '{print $1}'
}

# Uploads one file once under the folder, then reads it back through the public host.
publish() {
  local folder="$1" file="$2" scratch="$3" key state want got
  key="$folder/$(basename "$file")"
  if s3 put-object --key "$key" --body "$file" --content-type "$(content_type "$file")" \
    --cache-control "$CACHE_CONTROL" --if-none-match '*' > /dev/null 2> "$scratch/put.err"; then
    state=published
  elif grep -qF "(PreconditionFailed)" "$scratch/put.err"; then
    state="already published"
  else
    fail "could not upload $key: $(tail -1 "$scratch/put.err")"
  fi
  want="$(sha256_of "$file")"
  got="$(public_sha256 "$key")" || fail "could not read $PUBLIC_BASE/$key back"
  if [ "$got" != "$want" ]; then
    [ "$state" = published ] && fail "$PUBLIC_BASE/$key serves sha256 $got, not the uploaded $want"
    fail "$key already holds other bytes (sha256 $got), not $file's $want; a published file is never replaced"
  fi
  echo "publish_downloads: $state $PUBLIC_BASE/$key"
}

main() {
  [ $# -ge 2 ] || fail "usage: publish_downloads.sh linux/vX.Y.Z <file>..."
  local folder="$1" name file scratch
  shift
  [[ "$folder" =~ ^linux/v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "'$folder' is not a release's folder, linux/vX.Y.Z"
  for name in R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_ENDPOINT; do
    [ -n "${!name:-}" ] || fail "$name is required"
  done
  # Every file is checked before the first upload, so a release never publishes half of its files
  # for want of one.
  for file in "$@"; do
    [ -f "$file" ] || fail "no file at $file"
    content_type "$file" > /dev/null || fail "no content type for $file"
  done
  scratch="$(mktemp -d)"
  # shellcheck disable=SC2064 # the folder is named now, while it is in scope
  trap "rm -rf -- '$scratch'" EXIT
  for file in "$@"; do
    publish "$folder" "$file" "$scratch"
  done
}

main "$@"
