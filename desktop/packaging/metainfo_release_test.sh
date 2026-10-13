#!/usr/bin/env bash
# Offline tests for metainfo_release.py: the <release> it renders into the installed metainfo, for
# a release version and a development one, and what it refuses. The rendered files are validated
# pedantically by the appstreamcli the package's build uses: the private runtime's, run from its dev
# tree in the runtime's build image, with no network. A host's appstreamcli older than 1.0 does not
# know <developer> and is not asked.
#   FERMIX_RUNTIME_OUT=<runtime outputs> desktop/packaging/metainfo_release_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
metainfo="$here/../data/io.tezra.Fermix.metainfo.xml"
runtime="${FERMIX_RUNTIME_OUT:-${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime/out}"
image="${FERMIX_PACKAGE_IMAGE:-fermix-desktop-pkg-runtime-build:latest}"
container="fermix-desktop-pkg-package-metainfo-test-$$"
work=$(mktemp -d)
trap 'rm -rf "$work"; docker rm -f "$container" > /dev/null 2>&1 || true' EXIT

fail() {
  echo "metainfo_release_test: $*" >&2
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

render() {
  "$here/metainfo_release.py" --metainfo "${METAINFO:-$metainfo}" --version "$1" --date "$2" --out "$3"
}

# The runtime's appstreamcli over every file in <dir>, pedantically: "<file>: <exit status>" and its
# report, per file, into <dir>/reports.
validate_in_runtime() {
  local dir="$1"
  [ -f "$runtime/runtime-dev-amd64.tar" ] || fail "no runtime-dev-amd64.tar in $runtime: set FERMIX_RUNTIME_OUT"
  cp "$runtime/runtime-dev-amd64.tar" "$dir/"
  # shellcheck disable=SC2016 # expanded in the container
  docker create --name "$container" --network none --pull never --entrypoint bash "$image" -c '
    tar -xf /in/runtime-dev-amd64.tar -C / && /usr/lib/fermix-desktop/bin/appstreamcli --version
    for file in /in/*.xml; do
      status=0
      report="$(/usr/lib/fermix-desktop/bin/appstreamcli validate --pedantic --no-net "$file" 2>&1)" || status=$?
      printf "== %s %s\n%s\n" "$(basename "$file")" "$status" "$report"
    done' > /dev/null
  docker cp "$dir/." "$container:/in"
  docker start -a "$container" > "$dir/reports"
  rm "$dir/runtime-dev-amd64.tar"
}

# appstreamcli accepted <file>, and its only pedantic hint is the app id's capital letter.
validated() {
  local file="$1" reports="$2" report hints
  report="$(awk -v name="$file" '/^== / { inside = ($2 == name) } inside' "$reports")"
  [ -n "$report" ] || fail "appstreamcli did not report on $file"
  [[ "$(head -n 1 <<< "$report")" == "== $file 0" ]] || fail "appstreamcli refuses $file: $report"
  hints="$(grep -E '^[EWIP]: ' <<< "$report" | grep -vF 'cid-contains-uppercase-letter' || [ $? -eq 1 ])"
  [ -z "$hints" ] || fail "appstreamcli has more to say about $file: $hints"
}

! grep -qF '<releases' "$metainfo" || fail "the repository's metainfo carries a <releases> of its own"

echo "metainfo_release_test: a release version and a development one"
versions=(0.12.1 0.12.1+2 0.12.1+0.dev.20261005025130.92f91eb8695c.dirty)
mkdir -p "$work/rendered"
for version in "${versions[@]}"; do
  render "$version" 2026-10-05 "$work/rendered/$version.xml" > /dev/null
  grep -qxF "    <release version=\"$version\" date=\"2026-10-05\"/>" "$work/rendered/$version.xml" ||
    fail "the $version metainfo has no release line: $(grep -F release "$work/rendered/$version.xml")"
  [ "$(grep -c '<release ' "$work/rendered/$version.xml")" = 1 ] || fail "the $version metainfo has more than one release"
done
# The control: the repository's own file, which has no release, draws the hint the others must not.
cp "$metainfo" "$work/rendered/unreleased.xml"
validate_in_runtime "$work/rendered"
for version in "${versions[@]}"; do
  validated "$version.xml" "$work/rendered/reports"
  echo "  ok: $version, dated 2026-10-05, accepted with only the app id hint by $(head -n 1 "$work/rendered/reports")"
done
grep -qF 'releases-info-missing' "$work/rendered/reports" ||
  fail "the control drew no releases-info-missing: the hints were not read"
echo "  ok: the control without a release draws releases-info-missing"
# Without its releases block, a rendered file is the repository's, byte for byte.
block='\n  <releases>\n    <release version="0.12.1" date="2026-10-05"/>\n  </releases>\n'
python3 -c 'import sys; rendered, original, block = sys.argv[1:]
sys.exit(open(rendered).read().replace(block.encode().decode("unicode_escape"), "", 1) != open(original).read())' \
  "$work/rendered/0.12.1.xml" "$metainfo" "$block" || fail "rendering changed more than adding the releases block"
echo "  ok: nothing but the releases block, before </component>, is added"

echo "metainfo_release_test: what it refuses"
expect_refusal "a version with a Debian revision" "is not a package version" render 0.12.1-1 2026-10-05 "$work/x.xml"
expect_refusal "a version with an epoch" "is not a package version" render 1:0.12.1 2026-10-05 "$work/x.xml"
expect_refusal "a date that is not one" "2026-13-01 is not a date" render 0.12.1 2026-13-01 "$work/x.xml"
expect_refusal "a time rather than a date" "is not a date" render 0.12.1 2026-10-05T00:00:00Z "$work/x.xml"
cp "$work/rendered/0.12.1.xml" "$work/released.xml"
METAINFO="$work/released.xml" expect_refusal "a metainfo that already has releases" "already has <releases>" \
  render 0.12.1 2026-10-05 "$work/x.xml"
sed '/<\/component>/d' "$metainfo" > "$work/open.xml"
METAINFO="$work/open.xml" expect_refusal "a metainfo with no closing component" "one </component>" \
  render 0.12.1 2026-10-05 "$work/x.xml"
[ ! -e "$work/x.xml" ] || fail "a refused rendering left a file behind"
expect_refusal "no arguments" "usage" "$here/metainfo_release.py"

echo "metainfo_release_test: ok"
