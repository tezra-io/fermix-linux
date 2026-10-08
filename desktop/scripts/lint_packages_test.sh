#!/usr/bin/env bash
# Offline tests for lint_packages.sh: the command lines it refuses, how it judges a lintian and an
# rpmlint report, and that every lintian override and rpmlint filter shipped has its reason.
#   desktop/scripts/lint_packages_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
packaging="$(dirname "$here")/packaging"
lint="$here/lint_packages.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "lint_packages_test: $*" >&2
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

expect_pass() {
  local what="$1" said="$2" output
  shift 2
  output="$("$@" 2>&1)" || fail "$what was refused: $output"
  grep -qF -- "$said" <<< "$output" || fail "$what passed without saying '$said': $output"
  echo "  passed: $what"
}

echo "lint_packages_test: command lines it refuses"
touch "$work/fermix-desktop_0.0.1_amd64.deb" "$work/fermix-desktop-0.0.1-1.x86_64.rpm" "$work/other.deb"
deb="$work/fermix-desktop_0.0.1_amd64.deb"
rpm="$work/fermix-desktop-0.0.1-1.x86_64.rpm"
expect_refusal "no arguments" "usage" "$lint"
expect_refusal "no --out" "usage" "$lint" "$deb" "$rpm"
expect_refusal "a third package" "usage" "$lint" "$deb" "$rpm" "$rpm" --out "$work/out"
expect_refusal "an unknown option" "usage" "$lint" "$deb" "$rpm" --out "$work/out" --fast
expect_refusal "a deb that is not there" "no deb at" "$lint" "$work/none.deb" "$rpm" --out "$work/out"
expect_refusal "an rpm that is not there" "no rpm at" "$lint" "$deb" "$work/none.rpm" --out "$work/out"
expect_refusal "another package's deb" "is not a fermix-desktop deb" "$lint" "$work/other.deb" "$rpm" \
  --out "$work/out"
expect_refusal "the packages swapped" "is not a fermix-desktop deb" "$lint" "$rpm" "$deb" --out "$work/out"
expect_refusal "a third judge" "usage" "$lint" --judge piuparts "$work/report"
[ ! -e "$work/out" ] || fail "a refused lint left an output directory behind"

echo "lint_packages_test: lintian reports"
cat > "$work/lintian-clean.txt" <<'EOF'
Lintian v2.122.0
N: The runtime is private by design.
O: fermix-desktop: embedded-library expat [usr/lib/fermix-desktop/lib/libexpat.so.1.12.4]
O: fermix-desktop: embedded-library tiff [usr/lib/fermix-desktop/lib/libtiff.so.6.3.0]
O: fermix-desktop: statically-linked-binary [usr/bin/fermix]
EOF
expect_pass "only overridden tags" "3 overridden" "$lint" --judge lintian "$work/lintian-clean.txt"
expect_pass "the overridden tags counted" "2 embedded-library" "$lint" --judge lintian "$work/lintian-clean.txt"
for line in "E: fermix-desktop: no-changelog usr/share/doc/fermix-desktop/changelog.gz (native package)" \
  "W: fermix-desktop: no-manual-page [usr/bin/fermix-desktop]" \
  "I: fermix-desktop: unused-override hardening-no-pie [usr/lib/fermix-desktop/libexec/*]" \
  "P: fermix-desktop: repeated-path-segment lib [usr/lib/fermix-desktop/lib/]" \
  "X: fermix-desktop: executable-in-usr-lib [usr/lib/fermix-desktop/bin/fermix-desktop]"; do
  cat "$work/lintian-clean.txt" > "$work/lintian-left.txt"
  echo "$line" >> "$work/lintian-left.txt"
  expect_refusal "a ${line%%:*} tag left" "$line" "$lint" --judge lintian "$work/lintian-left.txt"
done
printf 'Lintian v2.122.0\n' > "$work/lintian-empty.txt"
expect_refusal "a report with no tag at all" "did not lint the deb" "$lint" --judge lintian "$work/lintian-empty.txt"
expect_refusal "no report" "no lintian report" "$lint" --judge lintian "$work/none.txt"

echo "lint_packages_test: rpmlint reports"
cat > "$work/rpmlint-clean.txt" <<'EOF'
2.8.0
============================ rpmlint session starts ============================
rpmlint: 2.8.0
configuration:
    /usr/lib/python3.14/site-packages/rpmlint/configdefaults.toml
rpmlintrc: /lint/fermix-desktop.rpmlintrc
checks: 32, packages: 1

 1 packages and 0 specfiles checked; 0 errors, 0 warnings, 1975 filtered, 0 badness; has taken 8.0 s
EOF
expect_pass "nothing left" "1975 filtered" "$lint" --judge rpmlint "$work/rpmlint-clean.txt"
for line in "fermix-desktop.x86_64: E: statically-linked-binary /usr/lib/fermix-desktop/bin/fermix-desktop" \
  "fermix-desktop.x86_64: W: zero-perms /usr/share/doc/fermix-desktop/copyright 0" \
  "fermix-desktop.x86_64: E: unused-rpmlintrc-filter \"no-changelogname-tag\"" \
  "(none): E: unable to load the rpmlintrc"; do
  sed "\$i $line" "$work/rpmlint-clean.txt" > "$work/rpmlint-left.txt"
  expect_refusal "a message left: ${line#*: }" "$line" "$lint" --judge rpmlint "$work/rpmlint-left.txt"
done
sed 's/ 0 errors, 0 warnings,/ 1 errors, 0 warnings,/' "$work/rpmlint-clean.txt" > "$work/rpmlint-count.txt"
expect_refusal "an error counted and not shown" "counts 1 errors and 0 warnings" \
  "$lint" --judge rpmlint "$work/rpmlint-count.txt"
sed 's/ 1 packages / 2 packages /' "$work/rpmlint-clean.txt" > "$work/rpmlint-two.txt"
expect_refusal "two packages checked" "checked 2 packages" "$lint" --judge rpmlint "$work/rpmlint-two.txt"
sed 's/ 1975 filtered,/ 0 filtered,/' "$work/rpmlint-clean.txt" > "$work/rpmlint-none.txt"
expect_refusal "nothing filtered" "did not lint the rpm" "$lint" --judge rpmlint "$work/rpmlint-none.txt"
grep -v 'specfiles checked' "$work/rpmlint-clean.txt" > "$work/rpmlint-cut.txt"
expect_refusal "a report with no summary" "no rpmlint summary" "$lint" --judge rpmlint "$work/rpmlint-cut.txt"

# Every override and filter sits under a comment, its reason: the first line of each block of
# them is a comment.
echo "lint_packages_test: every shipped override and filter has its reason"
reasonless() {
  # The rule goes through the environment: awk -v would read its backslashes as escapes.
  RULE="$2" awk '
    /^#/ { reasoned = 1; next }
    /^$/ { reasoned = 0; next }
    $0 ~ ENVIRON["RULE"] { if (!reasoned) print FILENAME ":" NR ": " $0; next }
    { print FILENAME ":" NR ": not a comment, a blank line or " ENVIRON["RULE"] ": " $0 }' "$1"
}
for pair in "lintian-overrides|^fermix-desktop: [a-z0-9-]+( |$)" "fermix-desktop.rpmlintrc|^addFilter\\(\"\\^fermix-desktop"; do
  file="$packaging/${pair%%|*}"
  bad="$(reasonless "$file" "${pair#*|}")"
  [ -z "$bad" ] || fail "lines with no reason above them:"$'\n'"$bad"
  echo "  ok: $(grep -cE "${pair#*|}" "$file") in $(basename "$file"), each block under its reason"
done
printf '# a reason\nfermix-desktop: tag-a\n\nfermix-desktop: tag-b\n' > "$work/overrides"
[ -n "$(reasonless "$work/overrides" "^fermix-desktop: [a-z0-9-]+( |$)")" ] ||
  fail "an override with no reason above it was not found"
echo "  refused: an override with no reason above it"

echo "lint_packages_test: ok"
