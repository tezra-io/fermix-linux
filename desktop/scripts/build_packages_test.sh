#!/usr/bin/env bash
# Offline tests for build_packages.sh: the app's version and how it is read, the version a build
# gets and the ones it refuses, a development build under CI, and the runtime inputs it refuses
# before anything is built. The engine is the fixture engine of fixtures/engine, verified with the
# stub cosign; the version order is asked of dpkg and rpm themselves, in the pinned ubuntu:22.04 and
# AlmaLinux 9 images, which have to be pulled already. Nothing here builds, runs nFPM or reaches the
# network.
#   desktop/scripts/build_packages_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/fixtures/engine"
lock="$here/../packaging/runtime/RUNTIME.lock.json"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export PATH="$fixtures/bin:$PATH"
unset CI SOURCE_DATE_EPOCH
export FERMIX_PACKAGE_CACHE="$work/cache"

COMMIT=0123456789abcdef0123456789abcdef01234567
DEB="$fixtures/fermix_0.0.1_amd64.deb"
RPM="$fixtures/fermix-0.0.1-1.x86_64.rpm"
DPKG_IMAGE="ubuntu:22.04@sha256:b8b6ee6aa931ecd9d0d952abc34dc0e5f7c6a30c6bb71b079fe399fde0329c02"
RPM_IMAGE="almalinux:9@sha256:9819dc675b67b595c2b59e42be7763fca1a8bb217fa5944e04daa22e9a64db16"

fail() {
  echo "build_packages_test: $*" >&2
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

expect_equal() {
  [ "$2" = "$3" ] || fail "$1 is '$2', not '$3'"
}

build() {
  "$here/build_packages.sh" "$@"
}

# <name> <manifest>: a copy of the script in a tree of its own, under an app manifest of the test's.
app_tree() {
  mkdir -p "$work/$1/desktop/scripts" "$work/$1/desktop/app"
  cp "$here/build_packages.sh" "$work/$1/desktop/scripts/"
  printf '%s\n' "$2" > "$work/$1/desktop/app/Cargo.toml"
  echo "$work/$1/desktop/scripts/build_packages.sh"
}

echo "build_packages_test: the app's version, from the [package] table of its manifest"
app="$(build --print-app-version)"
[[ "$app" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "the app's version is '$app', not X.Y.Z"
next="${app%.*}.$((${app##*.} + 1))"
echo "  ok: $app, from desktop/app/Cargo.toml"
manifest='[package]
name = "fermix-desktop"
version = "1.2.3"
edition.workspace = true

[dependencies.gtk]
version = "0.11"'
expect_equal "the version of a manifest with another table's version after it" \
  "$("$(app_tree tables "$manifest")" --print-app-version)" 1.2.3
echo "  ok: 1.2.3, and not the version of a later table"
expect_refusal "a version the workspace gives" "does not give its version as one line version = \"X.Y.Z\"" \
  "$(app_tree workspace "$(printf '[package]\nname = "fermix-desktop"\nversion.workspace = true')")" --print-app-version
expect_refusal "a prerelease version" "version = \"0.1.0-rc.1\"" \
  "$(app_tree prerelease "$(printf '[package]\nname = "fermix-desktop"\nversion = "0.1.0-rc.1"')")" --print-app-version
expect_refusal "a manifest whose [package] has no version" "does not give its version" \
  "$(app_tree none "$(printf '[package]\nname = "fermix-desktop"\n\n[lib]\nversion = "0.1.0"')")" --print-app-version
expect_refusal "two versions in [package]" "does not give its version as one line" \
  "$(app_tree two "$(printf '[package]\nversion = "0.1.0"\nversion = "0.2.0"')")" --print-app-version
expect_refusal "the app's version asked for with other flags" "usage" build --print-app-version --arch amd64

# The pin of the fixture release, as engine_pin.py writes it.
"$fixtures/fixture_release.sh" "$work/release" "$DEB" "$RPM" "$COMMIT" v0.0.1
FIXTURE_GH_RELEASE="$work/release" "$here/engine_pin.py" --pin "$work/pin.json" v0.0.1 > /dev/null
release() {
  build --pin "$work/pin.json" "$@"
}

echo "build_packages_test: the version of a release build"
expect_equal "a release over the pinned engine 0.0.1" "$(release --version "$app" --print-version)" "$app"
echo "  ok: $app, the app's, over the engine 0.0.1 the pin names"
expect_refusal "the pinned engine's version" "the version 0.0.1 is not the app's: desktop/app/Cargo.toml says $app" \
  release --version 0.0.1 --print-version
expect_refusal "the app's next version, before the manifest says it" "the version $next is not the app's" \
  release --version "$next" --print-version
expect_refusal "a rebuild of the app's version" "a release version is X.Y.Z, and $app+1 is not" \
  release --version "$app+1" --print-version
expect_refusal "a version with a Debian revision" "the version $app-1 has a '-'" \
  release --version "$app-1" --print-version
expect_refusal "a version with an epoch" "the version 1:$app has a ':'" \
  release --version "1:$app" --print-version
expect_refusal "a tag rather than a version" "a release version is X.Y.Z, and v$app is not" \
  release --version "v$app" --print-version
expect_refusal "a pin a build cannot trust" "is not one a build can trust" \
  build --pin "$work/absent.json" --version "$app" --print-version

echo "build_packages_test: a development build"
expect_refusal "a development build under CI" "CI" \
  env CI=true "$here/build_packages.sh" --dev "$DEB" "$RPM" --print-version
dev="$(SOURCE_DATE_EPOCH=1791072000 build --dev "$DEB" "$RPM" --print-version)"
pattern="^${app//./\\.}\\+dev\\.20261004000000\\.[0-9a-f]{12}(\\.dirty)?$"
[[ "$dev" =~ $pattern ]] || fail "the development version is $dev"
later="$(SOURCE_DATE_EPOCH=1791072060 build --dev "$DEB" "$RPM" --print-version)"
echo "  ok: $dev, from the app's version, the UTC time and this checkout's commit"

# <a> <b> ...: whether dpkg, or rpm, puts each a before its b, in the pinned images.
dpkg_orders() {
  docker run --rm --network none --pull never "$DPKG_IMAGE" sh -euc '
    while [ "$#" -gt 0 ]; do dpkg --compare-versions "$1" lt "$2" || { echo "dpkg: $1 is not before $2"; exit 1; }; shift 2; done
  ' sh "$@"
}
rpm_orders() {
  docker run --rm --network none --pull never "$RPM_IMAGE" sh -euc '
    while [ "$#" -gt 0 ]; do
      [ "$(rpm --eval "%{lua: print(rpm.vercmp(\"$1\", \"$2\"))}")" = -1 ] || { echo "rpm: $1 is not before $2"; exit 1; }
      shift 2
    done
  ' sh "$@"
}

echo "build_packages_test: dpkg and rpm order the versions the same way"
order=("$app" "$dev" "$later" "$next")
pairs=()
for ((i = 0; i + 1 < ${#order[@]}; i++)); do
  pairs+=("${order[i]}" "${order[i + 1]}")
done
dpkg_orders "${pairs[@]}" || fail "dpkg orders the versions otherwise"
rpm_orders "${pairs[@]}" || fail "rpm orders the versions otherwise"
echo "  ok: ${order[*]}, in that order, in both"
# The control: the same checks refuse a development build put before the release it builds on.
if dpkg_orders "$dev" "$app" > "$work/control"; then
  fail "dpkg put $dev before $app"
fi
grep -qxF "dpkg: $dev is not before $app" "$work/control" || fail "the dpkg control failed otherwise: $(cat "$work/control")"
if rpm_orders "$dev" "$app" > "$work/control"; then
  fail "rpm put $dev before $app"
fi
grep -qxF "rpm: $dev is not before $app" "$work/control" || fail "the rpm control failed otherwise: $(cat "$work/control")"
echo "  ok: and both refuse $dev before $app"

echo "build_packages_test: arguments it refuses"
expect_refusal "no arguments" "usage" "$here/build_packages.sh"
expect_refusal "both a release and a development build" "usage" build --version "$app" --dev "$DEB" "$RPM"
expect_refusal "a development build with one package" "usage" build --dev "$DEB"
expect_refusal "an unknown flag" "usage" build --version "$app" --sign
expect_refusal "a third architecture" "riscv64" release --version "$app" --arch riscv64 --print-version

echo "build_packages_test: runtime inputs it refuses before building"
runtime="$work/runtime"
mkdir -p "$runtime"
printf 'tree\n' > "$runtime/runtime-amd64.tar"
printf 'dev tree\n' > "$runtime/runtime-dev-amd64.tar"
printf '{"schema_version": 1, "archive": "runtime-licenses.tar.gz", "components": []}\n' \
  > "$runtime/runtime-licenses.json"
printf 'archive\n' > "$runtime/runtime-licenses.tar.gz"
jq -n --slurpfile lock "$lock" '{schema_version: 1, architecture: "amd64", lock: $lock[0]}' \
  > "$runtime/runtime-manifest.json"
runtime_variant() {
  cp -a "$runtime" "$work/$1"
  (cd "$work/$1" && eval "$2" && sha256sum runtime-amd64.tar runtime-dev-amd64.tar runtime-manifest.json \
    runtime-licenses.json runtime-licenses.tar.gz > SHA256SUMS && eval "${3:-true}")
  echo "$work/$1"
}
refuse_runtime() {
  expect_refusal "$1" "$2" build --dev "$DEB" "$RPM" --runtime "$3" --out "$work/out"
}
refuse_runtime "a runtime directory that is not there" "no runtime-amd64.tar in $work/absent" "$work/absent"
refuse_runtime "a runtime without its dev tree" "no runtime-dev-amd64.tar" \
  "$(runtime_variant no-dev true 'rm runtime-dev-amd64.tar')"
refuse_runtime "a runtime file its SHA256SUMS does not match" "do not match the runtime's SHA256SUMS" \
  "$(runtime_variant tampered true 'printf x >> runtime-amd64.tar')"
refuse_runtime "a runtime file its SHA256SUMS does not list" "SHA256SUMS does not list runtime-licenses.json" \
  "$(runtime_variant unlisted true "sed -i '/runtime-licenses.json/d' SHA256SUMS")"
refuse_runtime "a runtime without its licence archive" "no runtime-licenses.tar.gz" \
  "$(runtime_variant no-licenses true 'rm runtime-licenses.tar.gz')"
refuse_runtime "a runtime built from another lock file" "was built from another RUNTIME.lock.json" \
  "$(runtime_variant other-lock "jq '.lock.glibc_floor = \"2.17\"' runtime-manifest.json > m && mv m runtime-manifest.json")"
refuse_runtime "a runtime of another architecture" "is the arm64 runtime" \
  "$(runtime_variant arm64 "jq '.architecture = \"arm64\"' runtime-manifest.json > m && mv m runtime-manifest.json")"
[ ! -e "$work/out" ] || fail "a refused build left an output directory"

echo "build_packages_test: ok"
