#!/usr/bin/env bash
#
# Prove that a downloaded engine archive is the engine the pin names, and unpack
# it.
#
# This is the gate between "a file with the right name arrived" and "this is the
# engine build we decided to put inside our package". Four things have to agree,
# and each disagreement is its own sentence:
#
#   1. the archive's sha256 is the digest engine/PIN.json records. The .sha256
#      sidecar beside the archive is NOT what is checked: it is written by the
#      same release that wrote the archive, so a re-cut or replaced release
#      carries a sidecar that agrees with itself and with nothing we decided.
#      The pin is the only authority here.
#   2. cosign verifies the detached signature against the pinned certificate
#      identity and OIDC issuer, so the archive provably came out of the engine
#      repository's release workflow at that tag.
#   3. the archive unpacks into an empty staging directory under a hardened
#      reader that refuses every shape a tar can carry that a release tree may
#      not: an absolute path, a `..`, a second entry for one name, anything
#      under a symbolic link, a device node, a hard link, a setuid or setgid
#      mode, a file or a total larger than the caps below, a tree deeper or
#      wider than the caps below, and any root but `fermix_app_engine/`.
#   4. `engine-manifest.json` agrees with the pin and with what was unpacked:
#      its schema version, its source commit, its product version, its
#      distribution identity, its target and architecture, its certificate
#      identity, and its `tree_sha256` recomputed over the unpacked tree with
#      the engine's own canonical digest definition.
#
# The unpack lives here rather than in a script of its own because the rule that
# a file has no standing until it is verified is easier to hold when the only
# thing that writes the staging tree is the verifier. On any failure the staging
# directory is left empty.
#
# No network, no token, no `gh`: everything it needs is the pin and the files
# scripts/fetch_engine.sh already put on disk. That is what makes every refusal
# above provable offline in scripts/verify_engine_test.sh.
#
# Usage:
#   verify_engine.sh <pin.json> <download-dir> --staging <dir>
#                    [--target <target>]... [--cosign <binary>]
#   verify_engine.sh --local-archive <path> --dev --target <target>
#                    --staging <dir>
#
#   --staging   an empty directory, created if absent. On success it holds
#               <staging>/<target>/fermix_app_engine/ for each target verified.
#   --target    linux_x86_64 or linux_aarch64, repeatable. Defaults to both in
#               release mode; required, and exactly one, in --dev mode.
#   --cosign    the verifier to use. Defaults to cosign on PATH.
#   --dev       verify an unsigned, locally built archive named by
#               --local-archive: no cosign, no pin, no pinned digest. The
#               manifest and tree checks still run and the build id must be a
#               development build. The result is not release grade, it says so,
#               and it refuses to run under CI=true.
#   --max-file-bytes, --max-total-bytes, --max-entries
#               lower one of the unpack caps. A value above the default is
#               refused: these tighten, never loosen.
#
# Exit status: 0 when every requested target verified, 1 on any refusal.
set -euo pipefail

USAGE="usage: verify_engine.sh <pin.json> <download-dir> --staging <dir> [--target <target>]... [--cosign <binary>]"

# What a release tree may be at most. The engine's own packager holds the same
# depth and entry ceilings; the byte ceilings are this side's, and they are
# roughly five times the largest engine tree anyone has built, which is the
# point: a cap that is never reached is still what stops an archive that claims
# a terabyte from filling the runner's disk before anything is checked.
MAX_FILE_BYTES=536870912
MAX_TOTAL_BYTES=2147483648
MAX_ENTRIES=30000
MAX_DEPTH=32

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/engine_pin.sh
source "$ROOT_DIR/scripts/engine_pin.sh"

PIN=""
DOWNLOAD_DIR=""
STAGING_DIR=""
LOCAL_ARCHIVE=""
COSIGN_BIN=""
DEV=0
STAGING_OWNED=0
TARGETS=()

fail() {
  echo "verify_engine: $*" >&2
  clear_staging
  exit 1
}

# Nothing half written survives a refusal. The staging directory is this
# script's to fill and this script's to empty, and a caller that finds anything
# in it may rely on every check above having passed.
#
# Only ever what this run put there: a directory that already held files is
# refused by prepare_staging, before ownership is taken, and those files are
# left exactly where their owner put them.
clear_staging() {
  [ "$STAGING_OWNED" = "1" ] || return 0
  [ -n "$STAGING_DIR" ] && [ -d "$STAGING_DIR" ] || return 0
  find "$STAGING_DIR" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
}

lower_cap() {
  local name="$1" current="$2" wanted="$3"
  case "$wanted" in
    '' | *[!0-9]*) fail "$name needs a whole number of at most $current" ;;
  esac
  [ "$wanted" -ge 1 ] && [ "$wanted" -le "$current" ] ||
    fail "$name is $wanted and the cap is $current; a cap may be lowered, never raised"
  printf '%s\n' "$wanted"
}

parse_arguments() {
  local positional=()
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --cosign)
        [ "$#" -ge 2 ] || fail "--cosign needs a binary path"
        COSIGN_BIN="$2"
        shift 2
        ;;
      --staging)
        [ "$#" -ge 2 ] || fail "--staging needs a directory"
        STAGING_DIR="$2"
        shift 2
        ;;
      --local-archive)
        [ "$#" -ge 2 ] || fail "--local-archive needs a path"
        LOCAL_ARCHIVE="$2"
        shift 2
        ;;
      --target)
        [ "$#" -ge 2 ] || fail "--target needs a target"
        engine_pin_architecture "$2" >/dev/null || exit 1
        TARGETS+=("$2")
        shift 2
        ;;
      --dev)
        DEV=1
        shift
        ;;
      --max-file-bytes)
        [ "$#" -ge 2 ] || fail "--max-file-bytes needs a number"
        MAX_FILE_BYTES="$(lower_cap --max-file-bytes "$MAX_FILE_BYTES" "$2")"
        shift 2
        ;;
      --max-total-bytes)
        [ "$#" -ge 2 ] || fail "--max-total-bytes needs a number"
        MAX_TOTAL_BYTES="$(lower_cap --max-total-bytes "$MAX_TOTAL_BYTES" "$2")"
        shift 2
        ;;
      --max-entries)
        [ "$#" -ge 2 ] || fail "--max-entries needs a number"
        MAX_ENTRIES="$(lower_cap --max-entries "$MAX_ENTRIES" "$2")"
        shift 2
        ;;
      --*) fail "unknown argument '$1' ($USAGE)" ;;
      *)
        positional+=("$1")
        shift
        ;;
    esac
  done

  [ -n "$STAGING_DIR" ] || fail "--staging <dir> is required ($USAGE)"
  if [ "$DEV" = "1" ]; then
    parse_dev_arguments "${positional[@]+"${positional[@]}"}"
  else
    parse_release_arguments "${positional[@]+"${positional[@]}"}"
  fi
}

parse_dev_arguments() {
  [ "$#" -eq 0 ] ||
    fail "--dev takes no pin and no download directory, only --local-archive"
  [ -n "$LOCAL_ARCHIVE" ] || fail "--dev needs --local-archive <path>"
  [ -z "$COSIGN_BIN" ] || fail "--dev verifies nothing with cosign, so --cosign means nothing here"
  [ "${#TARGETS[@]}" -eq 1 ] ||
    fail "--dev verifies one archive, so exactly one --target is required"
  [ -f "$LOCAL_ARCHIVE" ] || fail "no archive at $LOCAL_ARCHIVE"
  [ "${CI:-}" = "true" ] &&
    fail "--dev is a developer's shortcut past the signature, and CI=true; a release rail verifies the signed archive"
  return 0
}

parse_release_arguments() {
  [ "$#" -eq 2 ] || fail "$USAGE"
  [ -z "$LOCAL_ARCHIVE" ] || fail "--local-archive is only verifiable with --dev"
  PIN="$1"
  DOWNLOAD_DIR="$2"
  [ "${#TARGETS[@]}" -gt 0 ] || TARGETS=("${ENGINE_PIN_TARGETS[@]}")
  [ -d "$DOWNLOAD_DIR" ] || fail "no download directory at $DOWNLOAD_DIR"
}

prepare_staging() {
  mkdir -p "$STAGING_DIR" || fail "cannot create the staging directory $STAGING_DIR"
  [ -z "$(find "$STAGING_DIR" -mindepth 1 -print -quit)" ] ||
    fail "the staging directory already holds files: $STAGING_DIR"
  STAGING_OWNED=1
}

resolve_cosign() {
  if [ -n "$COSIGN_BIN" ]; then
    [ -x "$COSIGN_BIN" ] || fail "the cosign binary is not executable: $COSIGN_BIN"
    return 0
  fi
  COSIGN_BIN="$(command -v cosign)" ||
    fail "cosign is not on PATH, so a pinned archive's signature cannot be checked; pass --cosign <binary>"
}

# One digest, computed the same way on every host this runs on. macOS ships
# shasum and no sha256sum; a Debian container ships sha256sum and no shasum.
digest_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

check_signature() {
  local target="$1" path="$2" asset expected actual
  asset="$(basename "$path")"

  [ -f "$path.sig" ] || fail "the pinned archive $asset has no $asset.sig beside it"
  [ -f "$path.pem" ] || fail "the pinned archive $asset has no $asset.pem beside it"

  expected="$(engine_pin_artifact_field "$PIN" "$target" sha256)" || exit 1
  actual="$(digest_of "$path")"
  [ "$actual" = "$expected" ] ||
    fail "$asset hashes to $actual, and the engine pin records $expected"

  "$COSIGN_BIN" verify-blob \
    --certificate "$path.pem" \
    --signature "$path.sig" \
    --certificate-identity "$IDENTITY" \
    --certificate-oidc-issuer "$ISSUER" \
    "$path" >/dev/null 2>&1 ||
    fail "$asset carries no signature from $IDENTITY issued by $ISSUER"
}

# The unpack and the manifest, in one python process: the reader that decides
# which members may exist is the same one that writes them, so there is no
# window in which an unchecked member is on disk.
unpack_and_check() {
  local target="$1" path="$2" mode="$3"
  unpack_engine_archive \
    "$path" "$STAGING_DIR/$target" "$target" "$mode" \
    "$COMMIT" "$VERSION" "$IDENTITY" \
    "$MAX_FILE_BYTES" "$MAX_TOTAL_BYTES" "$MAX_ENTRIES" "$MAX_DEPTH" ||
    fail "$(basename "$path") did not unpack as the engine release the manifest claims"
}

verify_target() {
  local target="$1" asset path
  asset="$(engine_pin_artifact_field "$PIN" "$target" asset)" || exit 1
  path="$DOWNLOAD_DIR/$asset"
  [ -f "$path" ] || fail "the pinned archive $asset is not in $DOWNLOAD_DIR"

  check_signature "$target" "$path"
  unpack_and_check "$target" "$path" release
  echo "verify_engine: $asset verified and unpacked ($target, $TAG, ${COMMIT:0:12})"
}

verify_local_archive() {
  local target="${TARGETS[0]}"
  echo "verify_engine: NOT RELEASE GRADE. --dev verifies $LOCAL_ARCHIVE with no signature and no pin." >&2
  echo "verify_engine: the manifest and the tree are checked; provenance is not. Never ship this." >&2
  unpack_and_check "$target" "$LOCAL_ARCHIVE" dev
  echo "verify_engine: $(basename "$LOCAL_ARCHIVE") unpacked ($target, development build, unsigned)"
}

run_release() {
  local target
  STATE="$(engine_pin_state "$PIN")" || exit 1
  [ "$STATE" = "pinned" ] ||
    fail "$PIN is unpinned, so there is no engine release to verify against"

  TAG="$(engine_pin_field "$PIN" tag)"
  VERSION="$(engine_pin_field "$PIN" engine_version)"
  COMMIT="$(engine_pin_field "$PIN" source_commit)"
  IDENTITY="$(engine_pin_field "$PIN" certificate_identity)"
  ISSUER="$(engine_pin_field "$PIN" certificate_oidc_issuer)"

  resolve_cosign
  for target in "${TARGETS[@]}"; do
    verify_target "$target"
  done
  echo "verify_engine: ok"
}

unpack_engine_archive() {
  python3 - "$@" <<'PY'
"""Unpack one app-engine archive under hard limits and check its manifest.

Nothing is written outside the staging directory this is handed, and nothing is
left in it if any check fails: the caller empties it, and this process writes
only under it.
"""
import hashlib
import json
import os
import stat
import sys
import tarfile

(
    archive_path,
    staging,
    target,
    mode,
    expected_commit,
    expected_version,
    expected_identity,
) = sys.argv[1:8]
MAX_FILE_BYTES, MAX_TOTAL_BYTES, MAX_ENTRIES, MAX_DEPTH = (int(v) for v in sys.argv[8:12])

ROOT = "fermix_app_engine"
MANIFEST_NAME = "engine-manifest.json"
MAX_NAME_BYTES = 1024
ARCHITECTURES = {"linux_x86_64": "x86_64", "linux_aarch64": "aarch64"}
SCHEMA_VERSION = 1
DISTRIBUTION_IDENTITY = "linux_package"


def refuse(sentence):
    sys.exit(f"verify_engine: {sentence}")


def relative_of(name):
    """The path inside the archive root, refusing every shape a release has not.

    An absolute name, a `..`, a `.`, an empty component and a name longer than
    the cap are each refused by name rather than normalised away, because a
    normalised traversal is a traversal that succeeded.
    """
    if len(name.encode("utf-8", "surrogateescape")) > MAX_NAME_BYTES:
        refuse(f"archive member name is longer than {MAX_NAME_BYTES} bytes")
    if name.startswith("/"):
        refuse(f"archive member '{name}' is an absolute path")
    parts = name.split("/")
    if parts and parts[-1] == "":
        parts = parts[:-1]
    if not parts or parts[0] != ROOT:
        refuse(f"archive member '{name}' is not under {ROOT}/, and that is the only root")
    for part in parts[1:]:
        if part in ("", ".", ".."):
            refuse(f"archive member '{name}' carries a '{part}' path component")
    if len(parts) - 1 > MAX_DEPTH:
        refuse(f"archive member '{name}' is deeper than {MAX_DEPTH} directories")
    return "/".join(parts[1:])


def check_kind(member, name):
    if member.islnk():
        refuse(f"archive member '{name}' is a hard link, and a release tree carries none")
    if member.ischr() or member.isblk() or member.isfifo() or member.isdev():
        refuse(f"archive member '{name}' is a device or a pipe, and a release tree carries none")
    if not (member.isdir() or member.isfile() or member.issym()):
        refuse(f"archive member '{name}' is not a file, a directory or a symbolic link")


def check_mode(member, name):
    if member.mode & (stat.S_ISUID | stat.S_ISGID):
        refuse(f"archive member '{name}' carries a setuid or setgid mode")
    if member.mode & stat.S_ISVTX:
        refuse(f"archive member '{name}' carries a sticky mode")
    if member.mode & ~0o777:
        refuse(f"archive member '{name}' carries mode bits outside the permission bits")


def check_parents(relative, directories, symlinks):
    """Every entry's parent is a directory this archive already declared.

    That refuses two things at once: an entry under a symbolic link, which is
    how a tar writes outside its root without ever naming `..`, and an entry
    whose parent directory is implied rather than carried, whose mode would
    otherwise be invented here and would not be the mode the digest was taken
    over.
    """
    parts = relative.split("/")
    for depth in range(1, len(parts)):
        parent = "/".join(parts[:depth])
        if parent in symlinks:
            refuse(f"archive member '{relative}' is under the symbolic link '{parent}'")
        if parent not in directories:
            refuse(f"archive member '{relative}' has no directory entry for '{parent}'")


def check_symlink(member, relative):
    link = member.linkname
    if not link or link.startswith("/"):
        refuse(f"symbolic link '{relative}' names the absolute target '{link}'")
    resolved = os.path.normpath(os.path.join(os.path.dirname(relative), link))
    if resolved == ".." or resolved.startswith("../"):
        refuse(f"symbolic link '{relative}' points outside the archive root")


def write_directory(path, member):
    os.mkdir(path, 0o700)
    return path, member.mode


def write_file(archive, member, path):
    source = archive.extractfile(member)
    if source is None:
        refuse(f"archive member '{member.name}' has no readable content")
    handle = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(handle, "wb") as destination:
            copied = 0
            while True:
                chunk = source.read(262144)
                if not chunk:
                    break
                copied += len(chunk)
                if copied > member.size:
                    refuse(f"archive member '{member.name}' carries more bytes than it declares")
                destination.write(chunk)
    finally:
        source.close()
    os.chmod(path, member.mode)


def extract(archive, root):
    """Write every member, refusing each one this reader may not write."""
    directories = {""}
    symlinks = set()
    seen = set()
    total = 0
    deferred = []

    for count, member in enumerate(archive, start=1):
        if count > MAX_ENTRIES:
            refuse(f"the archive carries more than {MAX_ENTRIES} entries")
        relative = relative_of(member.name)
        check_kind(member, member.name)
        check_mode(member, member.name)
        if relative == "":
            continue
        if relative in seen:
            refuse(f"archive member '{relative}' appears twice")
        seen.add(relative)
        check_parents(relative, directories, symlinks)

        path = os.path.join(root, relative)
        if member.isdir():
            deferred.append(write_directory(path, member))
            directories.add(relative)
        elif member.issym():
            check_symlink(member, relative)
            os.symlink(member.linkname, path)
            symlinks.add(relative)
        else:
            total += member.size
            if member.size > MAX_FILE_BYTES:
                refuse(f"archive member '{relative}' is larger than {MAX_FILE_BYTES} bytes")
            if total > MAX_TOTAL_BYTES:
                refuse(f"the archive unpacks to more than {MAX_TOTAL_BYTES} bytes")
            write_file(archive, member, path)

    # Directory modes last: a directory written read-only would refuse its own
    # children while the archive was still being read.
    for path, dir_mode in reversed(deferred):
        os.chmod(path, dir_mode)


def scan(root):
    """Every entry under the unpacked root, sorted, as the engine sorts them."""
    entries = []
    for directory, subdirectories, files in os.walk(root):
        subdirectories.sort()
        for name in sorted(subdirectories) + sorted(files):
            path = os.path.join(directory, name)
            entries.append((os.path.relpath(path, root), path))
    entries.sort(key=lambda entry: entry[0])
    return entries


def digest_record(relative, path):
    """The engine's canonical record, from scripts/release/package_app_engine.py."""
    metadata = os.lstat(path)
    if stat.S_ISLNK(metadata.st_mode):
        return f"symlink\0{relative}\0{0o777:04o}\0{os.readlink(path)}\n".encode()
    mode = f"{stat.S_IMODE(metadata.st_mode):04o}"
    if stat.S_ISDIR(metadata.st_mode):
        return f"directory\0{relative}\0{mode}\n".encode()
    return f"file\0{relative}\0{mode}\0{file_sha256(path)}\n".encode()


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(262144), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tree_digest(root):
    digest = hashlib.sha256()
    for relative, path in scan(root):
        if relative == MANIFEST_NAME:
            continue
        digest.update(digest_record(relative, path))
    return digest.hexdigest()


def load_manifest(root):
    path = os.path.join(root, MANIFEST_NAME)
    if os.path.islink(path) or not os.path.isfile(path):
        refuse(f"the archive carries no {MANIFEST_NAME} at its root")
    try:
        with open(path, encoding="utf-8") as handle:
            manifest = json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        refuse(f"{MANIFEST_NAME} is not readable JSON: {error}")
    if not isinstance(manifest, dict):
        refuse(f"{MANIFEST_NAME} is not a JSON object")
    return manifest


def section(manifest, name):
    value = manifest.get(name)
    if not isinstance(value, dict):
        refuse(f"{MANIFEST_NAME} carries no {name} object")
    return value


def check_manifest(manifest, root):
    if manifest.get("schema_version") != SCHEMA_VERSION:
        refuse(
            f"{MANIFEST_NAME} declares schema_version {manifest.get('schema_version')!r}, "
            f"and this reader understands {SCHEMA_VERSION}"
        )
    identity = section(manifest, "identity")
    check_identity(identity)
    check_provenance(section(manifest, "provenance"))

    actual = tree_digest(root)
    if manifest.get("tree_sha256") != actual:
        refuse(
            f"{MANIFEST_NAME} records tree_sha256 {manifest.get('tree_sha256')!r} "
            f"and what was unpacked digests to {actual}"
        )


def check_identity(identity):
    if identity.get("distribution_identity") != DISTRIBUTION_IDENTITY:
        refuse(
            f"the manifest's distribution_identity is "
            f"{identity.get('distribution_identity')!r}, and this rail carries "
            f"'{DISTRIBUTION_IDENTITY}'"
        )
    if identity.get("artifact_target") != target:
        refuse(
            f"the manifest's artifact_target is {identity.get('artifact_target')!r} "
            f"and this is the {target} archive"
        )
    if identity.get("architecture") != ARCHITECTURES[target]:
        refuse(
            f"the manifest's architecture is {identity.get('architecture')!r} "
            f"and {target} is {ARCHITECTURES[target]}"
        )
    if mode == "dev":
        check_development_build(identity)
        return
    commit = identity.get("source_commit")
    if not isinstance(commit, str) or commit.lower() != expected_commit.lower():
        refuse(
            f"the manifest's source_commit is {identity.get('source_commit')!r} "
            f"and the engine pin names {expected_commit}"
        )
    if identity.get("product_version") != expected_version:
        refuse(
            f"the manifest's product_version is {identity.get('product_version')!r} "
            f"and the engine pin names {expected_version}"
        )


def check_development_build(identity):
    """A dev verification may only ever look at a development build.

    The refusal that matters is the other direction: a release archive verified
    with --dev would be a release archive whose signature nobody checked, and
    the loud line this prints would then be attached to something that reads as
    release grade afterwards.
    """
    build_id = identity.get("build_id")
    if not isinstance(build_id, str) or not build_id:
        refuse("the manifest carries no build_id, so it cannot be a development build")
    if build_id.startswith("release-"):
        refuse(
            f"the manifest's build_id '{build_id}' is a release build, and --dev "
            "verifies no signature; verify it against the pin instead"
        )


def check_provenance(provenance):
    if mode == "dev":
        return
    if provenance.get("certificate_identity") != expected_identity:
        refuse(
            f"the manifest's certificate_identity is "
            f"{provenance.get('certificate_identity')!r} and the engine pin names "
            f"{expected_identity}"
        )


def main():
    if target not in ARCHITECTURES:
        refuse(f"'{target}' is not a target the engine publishes")
    if os.path.exists(staging):
        refuse(f"the staging directory for {target} already exists: {staging}")
    os.mkdir(staging, 0o755)
    root = os.path.join(staging, ROOT)
    os.mkdir(root, 0o755)

    try:
        with tarfile.open(archive_path, "r:gz") as archive:
            extract(archive, root)
    except tarfile.TarError as error:
        refuse(f"{os.path.basename(archive_path)} is not a readable tar.gz: {error}")
    except OSError as error:
        refuse(f"{os.path.basename(archive_path)} could not be unpacked: {error}")

    check_manifest(load_manifest(root), root)


main()
PY
}

parse_arguments "$@"
prepare_staging

if [ "$DEV" = "1" ]; then
  COMMIT=""
  VERSION=""
  IDENTITY=""
  verify_local_archive
else
  run_release
fi
