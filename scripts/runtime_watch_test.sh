#!/usr/bin/env bash
#
# Exercise runtime_watch.py against fixtures, with no network at all.
#
# The watcher is the whole answer to the CVE surface this amendment takes on
# (amendment section 4.5), and every way it can be wrong is quiet: a component
# it has no coordinate for, a version comparison that puts 4.9 above 4.16, a
# series rule that offers GTK 4.22 to a crate whose feature floor is 4.16, a
# source that could not be read reading as a component with nothing to report.
# Each of those is a case below.
#
# The two fixtures under scripts/fixtures/runtime_watch/ stand in for OSV and
# for release-monitoring.org, so this harness runs in milliseconds and answers
# the same way when both services are down.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/runtime_watch.py"
FIXTURES="$ROOT_DIR/scripts/fixtures/runtime_watch"
LOCK="$ROOT_DIR/packaging/runtime/RUNTIME.lock.json"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/runtime-watch-test.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT

fail() {
  echo "runtime_watch_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

# The watcher, against the two fixtures and whichever lock file the case wants.
watch() {
  python3 "$SCRIPT" \
    --lock "$1" \
    --out "$2" \
    --osv-fixture "$FIXTURES/osv.json" \
    --anitya-fixture "$FIXTURES/anitya.json"
}

# One field of one finding, read with python so the harness never parses JSON
# with a regular expression.
finding() {
  python3 - "$1" "$2" "$3" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    findings = json.load(handle)
for entry in findings:
    if entry["component"] == sys.argv[2]:
        print(json.dumps(entry[sys.argv[3]]))
        break
PY
}

echo "runtime_watch_test: the script parses"
python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" "$SCRIPT" ||
  fail "the script does not parse"
echo "  ok: python syntax"

echo "runtime_watch_test: refusals"
expect_refusal "no output directory" python3 "$SCRIPT" --lock "$LOCK"
expect_refusal "a lock file that does not exist" \
  python3 "$SCRIPT" --lock "$WORK/missing.json" --out "$WORK/out"

printf 'not json\n' > "$WORK/broken.json"
expect_refusal "a lock file that is not JSON" \
  python3 "$SCRIPT" --lock "$WORK/broken.json" --out "$WORK/out"

printf '{"schema_version": 1}\n' > "$WORK/empty.json"
expect_refusal "a lock file with no components" \
  python3 "$SCRIPT" --lock "$WORK/empty.json" --out "$WORK/out"

printf '{"components": [{"name": "gtk"}]}\n' > "$WORK/versionless.json"
expect_refusal "a component with no version" \
  python3 "$SCRIPT" --lock "$WORK/versionless.json" --out "$WORK/out"

expect_refusal "a timeout nobody would wait for" \
  python3 "$SCRIPT" --lock "$LOCK" --out "$WORK/out" --timeout 0
expect_refusal "a retry count with no cap" \
  python3 "$SCRIPT" --lock "$LOCK" --out "$WORK/out" --attempts 50

echo "runtime_watch_test: every component of the real lock file has a coordinate"
# The one refusal that cannot be a refusal: a component nobody watches has to be
# visible, so it is reported rather than skipped. This case asserts that the
# checked-in lock file currently leaves none of them unwatched.
watch "$LOCK" "$WORK/real" > "$WORK/real.log" || fail "the watcher refused the real lock file"
unwatched="$(python3 - "$WORK/real/findings.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    findings = json.load(handle)
print(" ".join(f["component"] for f in findings if f.get("unwatched")))
PY
)"
[ -z "$unwatched" ] ||
  fail "these components are watched by neither source: $unwatched"
echo "  ok: every component in RUNTIME.lock.json is watched"

echo "runtime_watch_test: the series rule"
# GTK is pinned to 4.16.x by the crate's feature floor, so 4.22.1 is not an
# upgrade and 4.16.9 is. A watcher that offered 4.22 every week would be
# ignored within a month.
[ "$(finding "$WORK/real/findings.json" gtk available)" = '"4.16.9"' ] ||
  fail "gtk was not offered 4.16.9 inside its own series"
echo "  ok: gtk 4.16.7 -> 4.16.9, and not 4.22.1"
[ "$(finding "$WORK/real/findings.json" libadwaita available)" = '"1.6.10"' ] ||
  fail "libadwaita was offered a version outside its pinned series, or none at all"
echo "  ok: libadwaita 1.6.9 -> 1.6.10, and not 1.8.2"

echo "runtime_watch_test: a component with no series pin takes the newest"
[ "$(finding "$WORK/real/findings.json" glib available)" = '"2.86.1"' ] ||
  fail "glib was not offered 2.86.1"
echo "  ok: glib 2.82.5 -> 2.86.1"

echo "runtime_watch_test: an advisory with no upgrade is still a finding"
[ "$(finding "$WORK/real/findings.json" libpng available)" = 'null' ] ||
  fail "libpng was offered an upgrade the fixture does not carry"
case "$(finding "$WORK/real/findings.json" libpng advisories)" in
  *OSV-2099-0001*) ;;
  *) fail "libpng carries no advisory, and the fixture gives it one" ;;
esac
grep -q 'runtime: libpng 1.6.58 advisories' "$WORK/real.log" ||
  fail "an advisory with no upgrade is not titled as one"
echo "  ok: libpng 1.6.58 is reported for its advisory alone"

echo "runtime_watch_test: a component with nothing to say opens nothing"
# libffi's fixture version is the pinned one and it carries no advisory. A
# watcher that reported it would open twenty-nine issues a week.
[ -z "$(finding "$WORK/real/findings.json" libffi component)" ] ||
  fail "libffi is a finding, and it has nothing to say"
[ ! -f "$WORK/real/libffi.md" ] ||
  fail "libffi has an issue body, and it has nothing to say"
echo "  ok: nothing to say, nothing written"

echo "runtime_watch_test: the issue body says what to do"
for needed in "RUNTIME.lock.json" "4.16.9" "Nothing is bumped automatically"; do
  grep -qF -- "$needed" "$WORK/real/gtk.md" ||
    fail "the gtk issue body does not carry '$needed'"
done
grep -qF "amendment section 4.4" "$WORK/real/gtk.md" ||
  fail "the gtk issue body does not say why the series is pinned"
echo "  ok: the body names the file, the version and the rule"

echo "runtime_watch_test: an unwatched component is a finding"
cat > "$WORK/stranger.json" <<'JSON'
{
  "schema_version": 1,
  "components": [
    { "name": "some-new-library", "version": "1.0.0" }
  ]
}
JSON
watch "$WORK/stranger.json" "$WORK/stranger" > "$WORK/stranger.log" ||
  fail "the watcher refused a lock file carrying an unknown component"
grep -q 'is not watched by either source' "$WORK/stranger.log" ||
  fail "an unwatched component was skipped rather than reported"
grep -q 'Add one' "$WORK/stranger/some-new-library.md" ||
  fail "the unwatched issue does not say what to do about it"
echo "  ok: a component neither source knows is reported, not skipped"


echo "runtime_watch_test: a purl in the lock file is what the upstream name comes from"
# The lock file is the authority on what a component is called. A component
# whose purl names a project the hand-written map does not is still watched, and
# a purl that disagrees with the map wins.
cat > "$WORK/purl.json" <<'JSON'
{
  "schema_version": 1,
  "components": [
    { "name": "some-new-library", "version": "1.0.0", "purl": "pkg:generic/glib" }
  ]
}
JSON
watch "$WORK/purl.json" "$WORK/purl" > "$WORK/purl.log" ||
  fail "the watcher refused a lock file carrying a purl"
[ "$(finding "$WORK/purl/findings.json" some-new-library available)" = '"2.86.1"' ] ||
  fail "the purl was not used to look the component up"
echo "  ok: pkg:generic/glib is looked up as glib, whatever the component is called"

echo "runtime_watch_test: every refusal fired"
