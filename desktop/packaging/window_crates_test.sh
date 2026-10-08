#!/usr/bin/env bash
# Offline tests for window_crates.py, over a cargo metadata document and a crate registry made up
# here: which crates the window's release build compiles, which of them are linked into it, and the
# licence texts read from each.
#   desktop/packaging/window_crates_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "window_crates_test: $*" >&2
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

registry="$work/registry"

# A crate in the registry: <name> <version> [licence file...]
crate() {
  local dir="$registry/$1-$2" file
  mkdir -p "$dir"
  printf '[package]\nname = "%s"\n' "$1" > "$dir/Cargo.toml"
  shift 2
  for file in "$@"; do
    printf 'The %s text of %s.\n' "$file" "$(basename "$dir")" > "$dir/$file"
  done
}

crate gtk4 0.11.0 LICENSE
crate glib 0.22.0 LICENSE COPYRIGHT.txt
crate serde 1.0.0 LICENSE-MIT LICENSE-APACHE
crate serde_derive 1.0.0 LICENSE-MIT
crate syn 2.0.0 LICENSE-MIT
crate cc 1.0.0 LICENSE-MIT
crate tempfile 3.0.0 LICENSE-MIT
crate earshot 1.2.2
crate base64 0.22.0 LICENSE-MIT
mkdir -p "$registry/oldstyle-0.1.0/docs"
printf '[package]\n' > "$registry/oldstyle-0.1.0/Cargo.toml"
printf 'The licence named by license_file.\n' > "$registry/oldstyle-0.1.0/docs/TERMS"

# The window (path) needs gtk4, serde and the path crate core; gtk4 needs glib; serde needs the
# proc-macro serde_derive, which needs syn; the window builds with cc and tests with tempfile;
# core needs earshot and base64, base64 also as a build dependency of glib, and oldstyle.
python3 - "$registry" > "$work/metadata.json" <<'PY'
import json
import sys

registry = sys.argv[1]
REG = "registry+https://github.com/rust-lang/crates.io-index"


def package(name, version, license, source=REG, kind="lib", license_file=None):
    path = f"{registry}/{name}-{version}" if source else f"/src/{name}"
    return {
        "name": name, "version": version, "id": f"{name} {version}", "license": license,
        "license_file": license_file, "source": source, "authors": [f"The {name} authors"],
        "repository": f"https://example.org/{name}", "manifest_path": f"{path}/Cargo.toml",
        "targets": [{"kind": [kind], "name": name}],
    }


def dep(name, version, *kinds):
    return {"name": name, "pkg": f"{name} {version}",
            "dep_kinds": [{"kind": kind, "target": None} for kind in kinds]}


packages = [
    package("fermix-desktop", "0.1.0", "MIT", source=None, kind="bin"),
    package("fermix-client", "0.1.0", "MIT", source=None),
    package("gtk4", "0.11.0", "MIT"),
    package("glib", "0.22.0", "MIT"),
    package("serde", "1.0.0", "MIT OR Apache-2.0"),
    package("serde_derive", "1.0.0", "MIT OR Apache-2.0", kind="proc-macro"),
    package("syn", "2.0.0", "MIT OR Apache-2.0"),
    package("cc", "1.0.0", "MIT OR Apache-2.0"),
    package("tempfile", "3.0.0", "MIT OR Apache-2.0"),
    package("earshot", "1.2.2", "MIT/Apache-2.0"),
    package("base64", "0.22.0", "MIT OR Apache-2.0"),
    package("oldstyle", "0.1.0", None, license_file="docs/TERMS"),
]
nodes = [
    {"id": "fermix-desktop 0.1.0", "deps": [
        dep("gtk4", "0.11.0", None), dep("serde", "1.0.0", None),
        dep("fermix-client", "0.1.0", None), dep("cc", "1.0.0", "build"),
        dep("tempfile", "3.0.0", "dev")]},
    {"id": "fermix-client 0.1.0", "deps": [
        dep("earshot", "1.2.2", None), dep("base64", "0.22.0", None),
        dep("oldstyle", "0.1.0", None)]},
    {"id": "gtk4 0.11.0", "deps": [dep("glib", "0.22.0", None)]},
    {"id": "glib 0.22.0", "deps": [dep("base64", "0.22.0", "build", None)]},
    {"id": "serde 1.0.0", "deps": [dep("serde_derive", "1.0.0", None)]},
    {"id": "serde_derive 1.0.0", "deps": [dep("syn", "2.0.0", None)]},
] + [{"id": f"{name} {version}", "deps": []} for name, version in (
    ("syn", "2.0.0"), ("cc", "1.0.0"), ("tempfile", "3.0.0"), ("earshot", "1.2.2"),
    ("base64", "0.22.0"), ("oldstyle", "0.1.0"))]
print(json.dumps({"packages": packages, "resolve": {"nodes": nodes, "root": None}, "version": 1}))
PY

crates() {
  "$here/window_crates.py" --metadata "$1" --root fermix-desktop --out "$2"
}

echo "window_crates_test: the crates the window's release build compiles"
crates "$work/metadata.json" "$work/crates.json" > /dev/null
names="$(jq -r '[.crates[] | .name + "-" + .version] | join(" ")' "$work/crates.json")"
want="base64-0.22.0 cc-1.0.0 earshot-1.2.2 glib-0.22.0 gtk4-0.11.0 oldstyle-0.1.0 serde-1.0.0"
want="$want serde_derive-1.0.0 syn-2.0.0"
[ "$names" = "$want" ] || fail "the crates are: $names; want: $want"
echo "  ok: $names"
echo "  ok: build dependencies, proc-macros and what they need; no dev crate, no path crate"
linked="$(jq -r '[.crates[] | select(.linked) | .name] | join(" ")' "$work/crates.json")"
[ "$linked" = "base64 earshot glib gtk4 oldstyle serde" ] || fail "the linked crates are: $linked"
echo "  ok: linked into the window: $linked"

# One jq expression over one crate's record: <crate> <expression>
field() {
  jq -c --arg name "$1" ".crates[] | select(.name == \$name) | $2" "$work/crates.json"
}

expect_field() {
  local have
  have="$(field "$1" "$2")"
  [ "$have" = "$3" ] || fail "$1's $2 is $have; want $3"
}

expect_field glib '[.files[].name]' '["COPYRIGHT.txt","LICENSE"]'
expect_field serde '.files[1].text' '"The LICENSE-MIT text of serde-1.0.0.\n"'
expect_field earshot '.license' '"MIT OR Apache-2.0"'
expect_field earshot '.files' '[]'
expect_field oldstyle '.license' '"LicenseRef-oldstyle-0.1.0"'
expect_field oldstyle '.files[0].name' '"docs/TERMS"'
expect_field gtk4 '.authors' '["The gtk4 authors"]'
echo "  ok: every licence file, its text, the licence with / read as OR, and a license_file"

echo "window_crates_test: what it refuses"
sed 's/"oldstyle", "version": "0.1.0", "id": "oldstyle 0.1.0", "license": null, "license_file": "docs\/TERMS"/"oldstyle", "version": "0.1.0", "id": "oldstyle 0.1.0", "license": null, "license_file": null/' \
  "$work/metadata.json" > "$work/no-licence.json"
expect_refusal "a crate with no licence at all" "oldstyle 0.1.0 declares no licence" \
  crates "$work/no-licence.json" "$work/x.json"
rm "$registry/oldstyle-0.1.0/docs/TERMS"
expect_refusal "a license_file the crate does not ship" "docs/TERMS" crates "$work/metadata.json" "$work/x.json"
expect_refusal "a root that is not in the metadata" "no package fermix-window" \
  "$here/window_crates.py" --metadata "$work/metadata.json" --root fermix-window --out "$work/x.json"
printf '{"packages": []' > "$work/truncated.json"
expect_refusal "metadata that is not JSON" "not valid JSON" crates "$work/truncated.json" "$work/x.json"
[ ! -e "$work/x.json" ] || fail "a refused run left a crate list behind"
expect_refusal "no arguments" "usage" "$here/window_crates.py"

echo "window_crates_test: ok"
