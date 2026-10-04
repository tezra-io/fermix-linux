#!/usr/bin/env bash
# Offline tests for verify_engine.py: a fixture release verifies and stages, and each way a release
# can be wrong is refused by the check that owns it, with no stage left behind.
#
# Signatures are checked by the stub cosign in fixtures/engine/bin, put first on PATH here; it refuses
# a certificate minted for another identity or issuer, and a signature over other bytes. The rpm half
# is read inside verify_engine.py's pinned AlmaLinux image with no network, so that image has to be
# pulled already. dpkg-deb and python3 come from the host. Nothing here reaches the network.
#   desktop/scripts/verify_engine_test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/fixtures/engine"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export PATH="$fixtures/bin:$PATH"

COMMIT=0123456789abcdef0123456789abcdef01234567
DEB="$fixtures/fermix_0.0.1_amd64.deb"
RPM="$fixtures/fermix-0.0.1-1.x86_64.rpm"

fail() {
  echo "verify_engine_test: $*" >&2
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

# A case: a fixture release around <amd64 deb> and <amd64 rpm>, the pin engine_pin.py writes from it,
# and the amd64 packages with their sidecars in a download directory, as fetch_engine.sh leaves them.
make_case() {
  local name="$1" deb="$2" commit="$3" signed_as="$4" rpm="${5:-$RPM}" assets="$work/$1/release/assets"
  "$fixtures/fixture_release.sh" "$work/$name/release" "$deb" "$rpm" "$commit" "$signed_as"
  FIXTURE_GH_RELEASE="$work/$name/release" "$here/engine_pin.py" --pin "$work/$name/pin.json" v0.0.1 > /dev/null
  mkdir "$work/$name/download"
  cp "$assets"/fermix_0.0.1_amd64.deb* "$assets"/fermix-0.0.1-1.x86_64.rpm* "$work/$name/download/"
}

verify_case() {
  "$here/verify_engine.py" --pin "$work/$1/pin.json" "$work/$1/download" "$work/$1/stage" "${2:-amd64}"
}

# Nothing of a refused verification survives: no stage, and no partial stage beside it.
expect_no_stage() {
  local parent="$1" what="$2"
  [ ! -e "$parent/stage" ] || fail "$what left a stage behind"
  [ -z "$(find "$parent" -maxdepth 1 -name '.stage.partial.*' -print -quit)" ] ||
    fail "$what left a partial stage behind"
}

refuse_case() {
  expect_refusal "$1" "$2" verify_case "$3"
  expect_no_stage "$work/$3" "$1"
}

# The fixture deb, unpacked, changed by one edit function run inside it, and rebuilt.
variant_deb() {
  local out="$1" edit="$2" root
  root="$(mktemp -d "$work/variant.XXXXXX")/root"
  dpkg-deb -R "$DEB" "$root"
  (cd "$root" && "$edit")
  dpkg-deb --root-owner-group -Zgzip -b "$root" "$out" > /dev/null
}

# The fixture deb with a data archive no dpkg-deb -b would build, assembled as an ar archive by hand:
# "escape" adds a file whose path climbs out of the tree, "user" gives every entry to uid 1000.
crafted_deb() {
  python3 - "$DEB" "$1" "$2" <<'PY2'
import gzip
import io
import subprocess
import sys
import tarfile

fixture, out, change = sys.argv[1:4]


def archive(flag):
    return subprocess.run(["dpkg-deb", flag, fixture], check=True, capture_output=True).stdout


def ar_member(name, content):
    header = f"{name:<16}{0:<12}{0:<6}{0:<6}{0o100644:<8o}{len(content):<10}`\n".encode()
    return header + content + (b"\n" if len(content) % 2 else b"")


data = io.BytesIO()
with tarfile.open(fileobj=io.BytesIO(archive("--fsys-tarfile"))) as source, tarfile.open(fileobj=data, mode="w") as target:
    for member in source:
        if change == "user":
            member.uid, member.gid, member.uname, member.gname = 1000, 1000, "someone", "someone"
        target.addfile(member, source.extractfile(member) if member.isfile() else None)
    if change == "escape":
        escaped = tarfile.TarInfo("./usr/../../escaped")
        escaped.size, escaped.mode = 2, 0o644
        target.addfile(escaped, io.BytesIO(b"x\n"))
with open(out, "wb") as handle:
    handle.write(b"!<arch>\n" + ar_member("debian-binary", b"2.0\n"))
    handle.write(ar_member("control.tar.gz", gzip.compress(archive("--ctrl-tarfile"))))
    handle.write(ar_member("data.tar.gz", gzip.compress(data.getvalue())))
PY2
}

edit_content() { printf 'one more line\n' >> usr/share/doc/fermix/copyright; }
edit_mode() { chmod 0755 usr/share/doc/fermix/copyright; }
edit_extra_file() {
  printf 'extra\n' > usr/share/fermix/extra
  chmod 0644 usr/share/fermix/extra
}
edit_postinst() { printf '# one more line\n' >> DEBIAN/postinst; }
edit_preinst() {
  printf '#!/bin/sh\nexit 0\n' > DEBIAN/preinst
  chmod 0755 DEBIAN/preinst
}
edit_symlink() { ln -s fermix usr/bin/fermix-link; }
edit_version() { sed -i 's/^Version: 0.0.1$/Version: 0.0.2/' DEBIAN/control; }
edit_depends() { sed -i '/^Architecture:/a Depends: libc6 (>= 2.34), libyaml-0-2 | libyaml' DEBIAN/control; }
edit_pre_depends() { sed -i '/^Architecture:/a Pre-Depends: dpkg (>= 1.19)' DEBIAN/control; }

# The stage holds the fixture engine's files with their modes, its maintainer scripts as shipped,
# and nothing else.
expect_stage_tree() {
  local stage="$1" path
  diff <(printf '%s\n' engine.json maintainer relations.json tree) <(ls -A "$stage") > "$work/diff" ||
    fail "the stage does not hold exactly tree, maintainer, relations.json and engine.json: $(cat "$work/diff")"
  diff <(printf '%s\n' '755 usr/bin/fermix' '644 usr/share/doc/fermix/copyright' \
    '644 usr/share/fermix/engine.json') <(cd "$stage/tree" && find . -type f -printf '%m %P\n' | sort -k2) > "$work/diff" ||
    fail "the staged tree is not the fixture's files and modes: $(cat "$work/diff")"
  [ -z "$(find "$stage/tree" ! -type f ! -type d -print -quit)" ] || fail "the staged tree holds a link"
  [ "$(stat -c %a "$stage" "$stage/tree" "$stage/maintainer")" = "755
755
755" ] || fail "the stage's own directories are not 0755"
  for path in usr/bin/fermix usr/share/fermix/engine.json usr/share/doc/fermix/copyright; do
    cmp -s "$fixtures/tree/$path" "$stage/tree/$path" || fail "the staged $path is not the packaged one"
  done
  cmp -s "$fixtures/maintainer/postinstall.sh" "$stage/maintainer/postinstall.sh" ||
    fail "the staged postinstall.sh is not the deb's postinst"
  cmp -s "$fixtures/maintainer/postremove.sh" "$stage/maintainer/postremove.sh" ||
    fail "the staged postremove.sh is not the deb's postrm"
  [ "$(stat -c %a "$stage/maintainer/postinstall.sh" "$stage/maintainer/postremove.sh")" = "755
755" ] || fail "the staged maintainer scripts are not executable"
}

# One Python assertion over the stage's engine.json and relations.json (`engine`, `relations`).
expect_record() {
  python3 - "$1" "$2" <<'PY'
import json
import sys

stage, assertion = sys.argv[1], sys.argv[2]
with open(f"{stage}/engine.json", encoding="utf-8") as handle:
    engine = json.load(handle)
with open(f"{stage}/relations.json", encoding="utf-8") as handle:
    relations = json.load(handle)
if not eval(assertion):
    sys.exit(f"verify_engine_test: the stage's record fails {assertion}:\n{engine}\n{relations}")
PY
}

digest() {
  sha256sum "$1" | cut -d' ' -f1
}

echo "verify_engine_test: a release that agrees with its pin"
make_case good "$DEB" "$COMMIT" v0.0.1
verify_case good > /dev/null
expect_stage_tree "$work/good/stage"
expect_record "$work/good/stage" "engine == {
  'schema_version': 1, 'dev': False, 'repository': 'tezra-io/fermix', 'tag': 'v0.0.1',
  'engine_version': '0.0.1', 'source_commit': '$COMMIT', 'build_id': 'release-1',
  'certificate_identity': 'https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/v0.0.1',
  'certificate_oidc_issuer': 'https://token.actions.githubusercontent.com', 'arch': 'amd64',
  'deb': {'asset': 'fermix_0.0.1_amd64.deb', 'sha256': '$(digest "$DEB")'},
  'rpm': {'asset': 'fermix-0.0.1-1.x86_64.rpm', 'sha256': '$(digest "$RPM")'}}"
expect_record "$work/good/stage" "relations == {
  'deb': {'asset': 'fermix_0.0.1_amd64.deb', 'depends': []},
  'rpm': {'asset': 'fermix-0.0.1-1.x86_64.rpm', 'requires': []}}"
echo "  ok: tree/, maintainer/, relations.json and engine.json"

make_case alias "$DEB" "$COMMIT" v0.0.1
mkdir "$work/alias/stage"
verify_case alias x86_64 > /dev/null
expect_stage_tree "$work/alias/stage"
echo "  ok: by the rpm's name for the architecture, into a stage directory that was there and empty"

echo "verify_engine_test: declared relations are carried as declared"
variant_deb "$work/depends.deb" edit_depends
make_case depends "$work/depends.deb" "$COMMIT" v0.0.1
verify_case depends > /dev/null
expect_record "$work/depends/stage" "relations['deb']['depends'] == ['libc6 (>= 2.34)', 'libyaml-0-2 | libyaml']"
echo "  ok: Depends: libc6 (>= 2.34), libyaml-0-2 | libyaml"
make_case requires "$DEB" "$COMMIT" v0.0.1 "$fixtures/requires/fermix-0.0.1-1.x86_64.rpm"
verify_case requires > /dev/null
expect_record "$work/requires/stage" "relations['rpm']['requires'] == ['/bin/sh', 'libc.so.6()(64bit)']"
echo "  ok: Requires: /bin/sh, libc.so.6()(64bit)"

echo "verify_engine_test: check 1, the sha256"
make_case tampered "$DEB" "$COMMIT" v0.0.1
printf 'one more byte' >> "$work/tampered/download/fermix_0.0.1_amd64.deb"
refuse_case "a package whose bytes are not the pinned ones" "sha256 check failed: fermix_0.0.1_amd64.deb hashes to" tampered

make_case agreeing-sidecar "$DEB" "$COMMIT" v0.0.1
printf 'one more byte' >> "$work/agreeing-sidecar/download/fermix-0.0.1-1.x86_64.rpm"
sha256sum "$work/agreeing-sidecar/download/fermix-0.0.1-1.x86_64.rpm" | sed 's|  .*/|  |' \
  > "$work/agreeing-sidecar/download/fermix-0.0.1-1.x86_64.rpm.sha256"
refuse_case "a changed package whose own sidecar agrees with it" "and the pin records" agreeing-sidecar

make_case sidecar "$DEB" "$COMMIT" v0.0.1
printf '%064d  fermix_0.0.1_amd64.deb\n' 0 > "$work/sidecar/download/fermix_0.0.1_amd64.deb.sha256"
refuse_case "a sidecar that disagrees with the pin and the file" "sha256 check failed: fermix_0.0.1_amd64.deb.sha256 records" sidecar

make_case misnamed "$DEB" "$COMMIT" v0.0.1
sed -i 's/fermix_0.0.1_amd64.deb/fermix_0.0.1_arm64.deb/' "$work/misnamed/download/fermix_0.0.1_amd64.deb.sha256"
refuse_case "a sidecar naming another file" "sha256 check failed: fermix_0.0.1_amd64.deb.sha256 names" misnamed

make_case no-rpm "$DEB" "$COMMIT" v0.0.1
rm "$work/no-rpm/download/fermix-0.0.1-1.x86_64.rpm"
refuse_case "a download without the rpm" "sha256 check failed: fermix-0.0.1-1.x86_64.rpm is not in" no-rpm

echo "verify_engine_test: check 2, the cosign identity"
make_case wrong-identity "$DEB" "$COMMIT" v9.9.9
refuse_case "packages signed by the release workflow at another tag" "cosign check failed: fermix_0.0.1_amd64.deb is not signed by" wrong-identity
grep -qF "none of the expected identities matched" "$work/stderr" || fail "cosign's own reason was not shown"

make_case wrong-signature "$DEB" "$COMMIT" v0.0.1
printf '%064d' 0 > "$work/wrong-signature/download/fermix-0.0.1-1.x86_64.rpm.sig"
refuse_case "a signature over other bytes" "cosign check failed: fermix-0.0.1-1.x86_64.rpm" wrong-signature

make_case unsigned "$DEB" "$COMMIT" v0.0.1
rm "$work/unsigned/download/fermix_0.0.1_amd64.deb.sig"
refuse_case "a package with no signature beside it" "cosign check failed: fermix_0.0.1_amd64.deb has no .sig" unsigned

echo "verify_engine_test: the pin"
make_case half "$DEB" "$COMMIT" v0.0.1
sed -i 's/"source_commit": "[0-9a-f]*"/"source_commit": null/' "$work/half/pin.json"
refuse_case "a half-filled pin" "is half filled: source_commit must be filled in" half

echo "verify_engine_test: the packages themselves"
variant_deb "$work/version.deb" edit_version
make_case version "$work/version.deb" "$COMMIT" v0.0.1
refuse_case "a deb whose control names another version" "package check failed: the deb's control says version 0.0.2" version

variant_deb "$work/preinst.deb" edit_preinst
make_case preinst "$work/preinst.deb" "$COMMIT" v0.0.1
refuse_case "a deb with a preinst the stage would drop" "package check failed: the deb carries the maintainer file preinst" preinst

variant_deb "$work/pre-depends.deb" edit_pre_depends
make_case pre-depends "$work/pre-depends.deb" "$COMMIT" v0.0.1
refuse_case "a deb with a Pre-Depends the stage would drop" "package check failed: the deb declares Pre-Depends" pre-depends

variant_deb "$work/symlink.deb" edit_symlink
make_case symlink "$work/symlink.deb" "$COMMIT" v0.0.1
refuse_case "a deb carrying a symbolic link" "package check failed: the deb carries the symbolic link usr/bin/fermix-link" symlink

make_case preinstall-rpm "$DEB" "$COMMIT" v0.0.1 "$fixtures/preinstall/fermix-0.0.1-1.x86_64.rpm"
refuse_case "an rpm with a %pre scriptlet the stage would drop" "package check failed: the rpm carries scriptlets or triggers beyond %post and %postun" preinstall-rpm

crafted_deb "$work/user.deb" user
mkdir "$work/user"
expect_refusal "a deb whose files are not root's" "package check failed: the deb carries usr owned by 1000:1000" \
  env -u CI "$here/verify_engine.py" --dev "$work/user/stage" "$work/user.deb"
expect_no_stage "$work/user" "a deb whose files are not root's"

crafted_deb "$work/escape.deb" escape
mkdir "$work/escape"
expect_refusal "a deb whose data climbs out of the tree" "package check failed: the deb carries the path ./usr/../../escaped" \
  env -u CI "$here/verify_engine.py" --dev "$work/escape/stage" "$work/escape.deb"
expect_no_stage "$work/escape" "a deb whose data climbs out of the tree"
[ -z "$(find "$work" -name escaped -print -quit)" ] || fail "a path that climbs out of the tree was written"

make_case other-commit "$DEB" abababababababababababababababababababab v0.0.1
refuse_case "packages built from another commit than the tag's" "package check failed: the engine was built from $COMMIT" other-commit

echo "verify_engine_test: the deb and the rpm hold the same engine"
variant_deb "$work/content.deb" edit_content
make_case content "$work/content.deb" "$COMMIT" v0.0.1
refuse_case "a deb whose file differs from the rpm's" "contents check failed: usr/share/doc/fermix/copyright" content

variant_deb "$work/mode.deb" edit_mode
make_case mode "$work/mode.deb" "$COMMIT" v0.0.1
refuse_case "a deb whose file mode differs from the rpm's" "contents check failed: usr/share/doc/fermix/copyright" mode

variant_deb "$work/extra.deb" edit_extra_file
make_case extra "$work/extra.deb" "$COMMIT" v0.0.1
refuse_case "a deb with a file the rpm lacks" "contents check failed: usr/share/fermix/extra" extra

variant_deb "$work/postinst.deb" edit_postinst
make_case postinst "$work/postinst.deb" "$COMMIT" v0.0.1
refuse_case "a deb whose postinst differs from the rpm's" "contents check failed: the deb's postinst" postinst

echo "verify_engine_test: the command line and the stage directory"
make_case occupied "$DEB" "$COMMIT" v0.0.1
mkdir "$work/occupied/stage"
touch "$work/occupied/stage/left-over"
expect_refusal "a stage that already holds files" "stage check failed: $work/occupied/stage already holds files" verify_case occupied
[ "$(ls -A "$work/occupied/stage")" = "left-over" ] || fail "a refused stage lost what was in it"

make_case arch "$DEB" "$COMMIT" v0.0.1
expect_refusal "an architecture nobody builds" "riscv64" verify_case arch riscv64
expect_no_stage "$work/arch" "an architecture nobody builds"
expect_refusal "too few arguments" "usage" "$here/verify_engine.py" "$work/good/download"

echo "verify_engine_test: --dev"
mkdir "$work/dev-ci"
expect_refusal "--dev with CI set" "dev check failed: CI is set" \
  env CI=true "$here/verify_engine.py" --dev "$work/dev-ci/stage" "$DEB" "$RPM"
expect_refusal "--dev with CI set and empty" "dev check failed: CI is set" \
  env CI= "$here/verify_engine.py" --dev "$work/dev-ci/stage" "$DEB"
expect_no_stage "$work/dev-ci" "--dev with CI set"

env -u CI "$here/verify_engine.py" --dev "$work/dev/stage" "$DEB" "$RPM" > /dev/null 2>&1
expect_stage_tree "$work/dev/stage"
expect_record "$work/dev/stage" "engine == {
  'schema_version': 1, 'dev': True, 'repository': None, 'tag': None,
  'engine_version': '0.0.1', 'source_commit': '$COMMIT', 'build_id': 'release-1',
  'certificate_identity': None, 'certificate_oidc_issuer': None, 'arch': 'amd64',
  'deb': {'asset': 'fermix_0.0.1_amd64.deb', 'sha256': '$(digest "$DEB")'},
  'rpm': {'asset': 'fermix-0.0.1-1.x86_64.rpm', 'sha256': '$(digest "$RPM")'}}"
echo "  ok: a local deb and rpm, stamped dev"

env -u CI "$here/verify_engine.py" --dev "$work/dev-deb/stage" "$DEB" > /dev/null 2>&1
expect_stage_tree "$work/dev-deb/stage"
expect_record "$work/dev-deb/stage" "engine['dev'] and engine['rpm'] is None and relations['rpm'] is None"
echo "  ok: a local deb alone, with no rpm to compare"

mkdir "$work/dev-mismatch"
expect_refusal "--dev with a deb and an rpm that differ" "contents check failed: usr/share/doc/fermix/copyright" \
  env -u CI "$here/verify_engine.py" --dev "$work/dev-mismatch/stage" "$work/content.deb" "$RPM"
expect_no_stage "$work/dev-mismatch" "--dev with a deb and an rpm that differ"

printf 'not a package\n' > "$work/garbage.deb"
mkdir "$work/dev-garbage"
expect_refusal "--dev with a file that is not a deb" "package check failed: $work/garbage.deb is not a deb" \
  env -u CI "$here/verify_engine.py" --dev "$work/dev-garbage/stage" "$work/garbage.deb"
expect_no_stage "$work/dev-garbage" "--dev with a file that is not a deb"

echo "verify_engine_test: ok"
