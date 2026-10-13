#!/usr/bin/env bash
#
# Lints a built pair of fermix-desktop packages with each family's own tool: lintian over the deb
# on Debian 13, and rpmlint --strict over the rpm on Fedora 44, each image pinned by digest here
# with the tool's version. The deb carries its own overrides (desktop/packaging/lintian-overrides);
# the rpm is linted with desktop/packaging/fermix-desktop.rpmlintrc. Every tag left is refused, and
# so is every override or filter that covers nothing, which lintian reports as unused-override and
# rpmlint as unused-rpmlintrc-filter.
#
#   lint_packages.sh <deb> <rpm> --out <dir>
#   lint_packages.sh --judge <lintian|rpmlint> <report>
#
# <dir> gets each tool's report (lintian.txt, rpmlint.txt), what it printed to stderr (.log) and
# the sha256 of the packages linted. --judge reads a report already written, as the lint does.
#
# Each package goes into its container by docker cp and is checked there against its sha256; each
# container is removed on every exit. The images reach their mirrors for the tools.
set -euo pipefail
shopt -s inherit_errexit

DEBIAN_IMAGE="debian:13@sha256:913f6706df59a68922d1dd08f78c2476560a8d367897200a6005b00e5f67c2d5"
LINTIAN_PACKAGE="lintian=2.122.0"
FEDORA_IMAGE="fedora:44@sha256:43b29f65a41eb9c35e1cd5323e3bdf3b655c2357a9f4f1ff2f9c2798e5045d80"
RPMLINT_PACKAGE="rpmlint-2.8.0-3.fc44"
# A lint that has not finished by then has failed; the container is removed either way.
LINT_TIMEOUT_SECONDS=1200
here=$(cd "$(dirname "$0")" && pwd)
desktop=$(dirname "$here")

# What the EXIT trap removes: it runs after every function has returned.
CONTAINER=""

fail() {
  echo "lint_packages: $*" >&2
  exit 1
}

usage() {
  fail "usage: lint_packages.sh <deb> <rpm> --out <dir> | --judge <lintian|rpmlint> <report>"
}

cleanup() {
  [ -z "$CONTAINER" ] || docker rm -f "$CONTAINER" > /dev/null 2>&1 ||
    echo "lint_packages: could not remove the container $CONTAINER" >&2
}

# A lintian report passes when no tag is left and the overrides covered some: a report with neither
# is one where lintian never read the package.
judge_lintian() {
  local report="$1" left overridden
  [ -f "$report" ] || fail "no lintian report at $report"
  left="$(grep -E '^[EWIPXC]: ' "$report" || [ $? -eq 1 ])"
  [ -z "$left" ] || fail "lintian has tags no override covers, in $report:"$'\n'"$left"
  overridden="$(grep -c '^O: ' "$report" || [ $? -eq 1 ])"
  [ "$overridden" -gt 0 ] || fail "$report has no overridden tag: lintian did not lint the deb"
  echo "lintian: no tag left; $overridden overridden, each with its reason:"
  grep '^O: ' "$report" | awk '{ print $3 }' | sort | uniq -c | awk '{ printf "  %5d %s\n", $1, $2 }'
}

# An rpmlint report passes when no message is left, its summary counts one package, no error and
# no warning, and the filters covered some.
judge_rpmlint() {
  local report="$1" left summary packages errors warnings filtered
  [ -f "$report" ] || fail "no rpmlint report at $report"
  left="$(grep -E ': [EWI]: ' "$report" || [ $? -eq 1 ])"
  [ -z "$left" ] || fail "rpmlint has messages no filter covers, in $report:"$'\n'"$left"
  summary="$(sed -nE 's/^.*\b([0-9]+) packages and [0-9]+ specfiles checked; ([0-9]+) errors, ([0-9]+) warnings, ([0-9]+) filtered.*$/\1 \2 \3 \4/p' "$report")"
  [ "$(wc -l <<< "$summary")" = 1 ] && [ -n "$summary" ] || fail "$report has no rpmlint summary"
  read -r packages errors warnings filtered <<< "$summary"
  [ "$packages" = 1 ] || fail "rpmlint checked $packages packages, not the one rpm"
  [ "$errors" = 0 ] && [ "$warnings" = 0 ] ||
    fail "rpmlint counts $errors errors and $warnings warnings that $report does not show"
  [ "$filtered" -gt 0 ] || fail "$report has nothing filtered: rpmlint did not lint the rpm"
  echo "rpmlint: no message left; $filtered filtered, by Fedora's own configuration and ours"
}

check_packages() {
  local deb="$1" rpm="$2"
  [ -f "$deb" ] || fail "no deb at $deb"
  [ -f "$rpm" ] || fail "no rpm at $rpm"
  [[ "$(basename -- "$deb")" == fermix-desktop_*.deb ]] || fail "$deb is not a fermix-desktop deb"
  [[ "$(basename -- "$rpm")" == fermix-desktop-*.rpm ]] || fail "$rpm is not a fermix-desktop rpm"
}

# Runs <script> in <image> with <files> copied in at /lint and checked there by sha256; its stdout
# is <out>/<tool>.txt and its stderr <out>/<tool>.log. The container's status, which is the tool's,
# has to be one of <accepted>.
lint_in() {
  local tool="$1" image="$2" out="$3" accepted="$4" script="$5" status=0 file dir
  shift 5
  local copy=(-C "$(cd "$out" && pwd)" "$tool.sha256")
  for file in "$@"; do
    dir="$(cd "$(dirname -- "$file")" && pwd)"
    (cd "$dir" && sha256sum -- "$(basename -- "$file")")
    copy+=(-C "$dir" "$(basename -- "$file")")
  done > "$out/$tool.sha256"
  docker create --name "fermix-desktop-pkg-package-$tool-$$" --pull missing "$image" \
    bash -euc "cd /lint && sha256sum --quiet --check $tool.sha256 && $script" > /dev/null
  CONTAINER="fermix-desktop-pkg-package-$tool-$$"
  tar -cf - --transform 'flags=r;s,^,lint/,' "${copy[@]}" | docker cp - "$CONTAINER:/"
  timeout "$LINT_TIMEOUT_SECONDS" docker start --attach "$CONTAINER" > "$out/$tool.txt" 2> "$out/$tool.log" ||
    status=$?
  docker rm -f "$CONTAINER" > /dev/null
  CONTAINER=""
  [[ " $accepted " == *" $status "* ]] ||
    fail "$tool could not lint $(basename -- "$1") (status $status); see $out/$tool.log"
}

# lintian runs as nobody, as it asks to be run, and exits 0 whatever it reports.
run_lintian() {
  local deb="$1" out="$2" name
  name="$(basename -- "$deb")"
  lint_in lintian "$DEBIAN_IMAGE" "$out" 0 "apt-get -qq update && DEBIAN_FRONTEND=noninteractive \
    apt-get -qq install --yes --no-install-recommends $(printf %q "$LINTIAN_PACKAGE") > /dev/null &&
    lintian --version && runuser -u nobody -- env HOME=/tmp lintian --fail-on none \
    --display-experimental --display-info --pedantic --show-overrides --tag-display-limit 0 \
    /lint/$(printf %q "$name")" "$deb"
}

# rpmlint exits 64 when it reports anything, which the judge then names.
run_rpmlint() {
  local rpm="$1" out="$2" name
  name="$(basename -- "$rpm")"
  lint_in rpmlint "$FEDORA_IMAGE" "$out" "0 64" "dnf -q -y install $(printf %q "$RPMLINT_PACKAGE") \
    > /dev/null && rpmlint --version && rpmlint --strict --rpmlintrc /lint/fermix-desktop.rpmlintrc \
    /lint/$(printf %q "$name")" "$rpm" "$desktop/packaging/fermix-desktop.rpmlintrc"
}

main() {
  local deb="" rpm="" out=""
  if [ "${1:-}" = --judge ]; then
    [ $# = 3 ] || usage
    case "$2" in
      lintian) judge_lintian "$3" ;;
      rpmlint) judge_rpmlint "$3" ;;
      *) usage ;;
    esac
    return 0
  fi
  while [ $# -gt 0 ]; do
    case "$1" in
      --out) [ $# -ge 2 ] || usage; out="$2"; shift 2 ;;
      -*) usage ;;
      *) if [ -z "$deb" ]; then deb="$1"; elif [ -z "$rpm" ]; then rpm="$1"; else usage; fi; shift ;;
    esac
  done
  [ -n "$deb" ] && [ -n "$rpm" ] && [ -n "$out" ] || usage
  check_packages "$deb" "$rpm"
  mkdir -p "$out"
  run_lintian "$deb" "$out"
  run_rpmlint "$rpm" "$out"
  judge_lintian "$out/lintian.txt"
  judge_rpmlint "$out/rpmlint.txt"
}

trap cleanup EXIT
main "$@"
