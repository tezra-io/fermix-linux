#!/usr/bin/env bash
#
# fermix-desktop's copyright file agrees with what the package is built from (A§4.6).
#
# Two checks, the second independent of the program that writes the file:
#
#   1. runtime-licenses.json accounts for every component of the lock, with its version and
#      licence, and names no other. Read back as Debian's format, the file has a paragraph for
#      every runtime component that is not build-only, every crate compiled into the runtime,
#      the Rust standard library, every crate the window's build compiles, every vendored part,
#      this repository and the engine, each with the licence its input names, and no paragraph
#      for anything else: a build-only component ships nothing and has none. Every licence a
#      paragraph states without a text has a License paragraph that gives it, and the engine's
#      paragraph points to the engine's own copyright.
#   2. The file is byte for byte what desktop/packaging/copyright.py writes from these inputs,
#      which holds every licence text to its source too.
#
# Every disagreement is printed before it refuses.
#
# Usage: check_copyright.sh <copyright file> --repo <dir> --lock <file> --runtime-licenses <file>
#          --runtime-texts <dir> --window-crates <file> --vendored <file> --standard-texts <dir>
#          --engine-copyright <file>
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
GENERATOR="$here/../packaging/copyright.py"
FLAGS=(repo lock runtime-licenses runtime-texts window-crates vendored standard-texts
  engine-copyright)
ENGINE_COPYRIGHT=/usr/share/doc/fermix/copyright

fail() {
  echo "check_copyright: $*" >&2
  exit 1
}

usage() {
  fail "usage: check_copyright.sh <copyright file> $(printf -- '--%s <path> ' "${FLAGS[@]}")"
}

# Every (pattern, licence) the inputs call for: "pattern<TAB>licence" per line. A runtime crate's
# Cargo-style "/" is read as OR, as copyright.py and window_crates.py read it.
expected() {
  local -n inputs=$1
  printf '*\tMIT\nengine/*\tMIT\n'
  jq -r '(.components[] | select(.build_only | not) | "runtime/\(.name)-\(.version)/*\t\(.license)"),
    (.crates[] | "runtime/crates/\(.name)-\(.version)/*\t\(.license | gsub("\\s*/\\s*"; " OR "))"),
    (.rust_std | "runtime/rust-std-\(.version)/*\t\(.license)")' "${inputs[runtime-licenses]}"
  jq -r '.crates[] | "crates/\(.name)-\(.version)/*\t\(.license)"' "${inputs[window-crates]}"
  jq -r '.vendored[] | .license as $licence | .files[] | "\(.)\t\($licence)"' "${inputs[vendored]}"
}

# The lock's components against runtime-licenses.json's, as one sentence per disagreement:
# <lock> <runtime-licenses.json>
accounting() {
  awk -F '\t' '
    FNR == NR { lock[$1 " " $2] = $3; next }
    { listed[$1 " " $2] = $3 }
    !(($1 " " $2) in lock) { print "runtime-licenses.json has the component " $1 " " $2 ", which the lock does not" }
    (($1 " " $2) in lock) && lock[$1 " " $2] != $3 {
      print "the lock gives " $1 " " $2 " the licence " lock[$1 " " $2] ", and runtime-licenses.json " $3
    }
    END { for (c in lock) if (!(c in listed)) print "the lock\047s component " c " is not in runtime-licenses.json" }' \
    <(jq -r '.components[] | [.name, .version, .license] | @tsv' "$1") \
    <(jq -r '.components[] | [.name, .version, .license] | @tsv' "$2") | sort
}

# The build-only components, as "name version, name version".
build_only() {
  jq -r '[.components[] | select(.build_only) | "\(.name) \(.version)"] | join(", ")' "$1"
}

# The file read as Debian's format: "FILES<TAB>pattern<TAB>licence<TAB>has text" per pattern of
# each Files paragraph, and "LICENSE<TAB>name<TAB>has text" per stand-alone License paragraph.
paragraphs() {
  awk 'BEGIN { RS = ""; FS = "\n" }
    $1 ~ /^License: / { print "LICENSE\t" substr($1, 10) "\t" (NF > 1 && $2 ~ /^ /); next }
    $1 ~ /^Files: / {
      files = substr($1, 8)
      for (i = 2; i <= NF && $i ~ /^ /; i++) files = files " " substr($i, 2)
      licence = ""; text = 0
      for (; i <= NF; i++) {
        if ($i ~ /^License: /) { licence = substr($i, 10); text = (i < NF && $(i + 1) ~ /^ /) }
      }
      n = split(files, patterns, " ")
      for (j = 1; j <= n; j++) print "FILES\t" patterns[j] "\t" licence "\t" text
    }' "$1"
}

# Claim 1, as one sentence per disagreement.
coverage() {
  local copyright="$1" want="$2" have="$3"
  awk -F '\t' '
    FNR == NR { want[$1] = $2; next }
    $1 == "FILES" { seen[$2] = $3; if (!($2 in want)) print "the file names " $2 ", which no input has" }
    $1 == "FILES" && $4 == 0 { bare[$3] = 1 }
    $1 == "LICENSE" && $3 == 1 { texts[$2] = 1 }
    END {
      for (p in want) if (!(p in seen) || seen[p] != want[p]) print "no paragraph for " p " with License: " want[p]
      for (l in bare) {
        n = split(l, ids, /[ ()]+/)
        for (i = 1; i <= n; i++) {
          id = ids[i]
          if (id != "" && id != "OR" && id != "AND" && id != "WITH" && !(id in texts)) print "License: " id " has no text"
        }
      }
    }' "$want" "$have" | sort -u
  awk -v pointer="$ENGINE_COPYRIGHT" 'BEGIN { RS = "" }
    /^Files: engine\/\*/ && index($0, pointer) { found = 1 }
    END { if (!found) print "the engine paragraph does not point to " pointer }' "$copyright"
}

main() {
  [ $# -ge 1 ] || usage
  local copyright="$1" name work problems lines skipped args=()
  shift
  declare -A given=()
  while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || usage
    name="${1#--}"
    [[ " ${FLAGS[*]} " == *" $name "* ]] || usage
    given[$name]="$2"
    shift 2
  done
  for name in "${FLAGS[@]}"; do
    [ -n "${given[$name]:-}" ] || usage
    args+=("--$name" "${given[$name]}")
  done
  [ -f "$copyright" ] || fail "no copyright file at $copyright"
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # the path is fixed now, and the directory goes on every exit
  trap "rm -rf -- '$work'" EXIT
  expected given > "$work/want"
  paragraphs "$copyright" > "$work/have"
  problems="$(accounting "${given[lock]}" "${given[runtime-licenses]}")"
  problems+=$'\n'"$(coverage "$copyright" "$work/want" "$work/have")"
  if ! python3 "$GENERATOR" "${args[@]}" --out "$work/generated" > /dev/null; then
    problems+=$'\n'"copyright.py cannot generate a copyright file from these inputs, as it says above"
  elif ! cmp -s "$copyright" "$work/generated"; then
    # diff exits 1 for files that differ, which they do here; 2 is a failure.
    diff "$work/generated" "$copyright" > "$work/diff" || [ $? -eq 1 ] || fail "diff could not compare the files"
    problems+=$'\n'"$copyright is not what copyright.py generates from its inputs; it differs at: $(
      head -n 4 "$work/diff" | tr '\n' ' ')"
  fi
  problems="$(sed '/^$/d' <<< "$problems")"
  if [ -n "$problems" ]; then
    mapfile -t lines <<< "$problems"
    printf 'check_copyright: %s\n' "${lines[@]}" >&2
    fail "the copyright file disagrees with its inputs in $(wc -l <<< "$problems") ways"
  fi
  skipped="$(build_only "${given[runtime-licenses]}")"
  echo "check_copyright: $(wc -l < "$work/want") parts, each in its paragraph with its licence," \
    "and the file is what copyright.py writes; build-only, with no paragraph: ${skipped:-none}"
}

main "$@"
