#!/usr/bin/env bash
#
# Assemble the two maintainer scripts the package ships, each from two
# fragments (amendment section 3.3).
#
# The engine half has to be byte-identical to the `fermix` package's, because
# "one engine layout, one Doctor, one set of maintainer-script steps" is a claim
# the design makes and a claim a person can check. The desktop half is this
# repository's two cache refreshes. Concatenating two shell scripts does not
# work on its own: the engine's ends in `exit 0`, which would stop the desktop
# half from ever running.
#
# So each fragment is wrapped in a subshell and neither is edited. Inside `( )`
# an `exit 0` ends the subshell and not the script, the fragment's own shebang
# is a comment, and its own `set -eu` applies where it was written to apply. The
# outer `set -eu` then turns a fragment that exits non-zero into a failed
# maintainer script, which is what dpkg and rpm are told about.
#
# The markers around each fragment are what `--check` reads back: it takes the
# bytes between them and compares them with the file they came from, so "the
# engine half is unchanged" is a gate rather than a sentence.
#
# Usage:
#   assemble_maintainer.sh --engine <dir> --desktop <dir> --out <dir>
#   assemble_maintainer.sh --engine <dir> --desktop <dir> --out <dir> --check
#
#   --engine   the unpacked artifact's maintainer/ directory
#   --desktop  packaging/scripts/
#   --out      where postinstall.sh and postremove.sh are written
#   --check    write nothing; refuse unless what is in --out is what this would
#              write, and unless each fragment inside it is its source verbatim
set -euo pipefail

ENGINE_DIR=""
DESKTOP_DIR=""
OUT_DIR=""
CHECK=0

fail() {
  echo "assemble_maintainer: $*" >&2
  exit 1
}

parse_arguments() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --engine)
        [ "$#" -ge 2 ] || fail "--engine needs a directory"
        ENGINE_DIR="$2"
        shift 2
        ;;
      --desktop)
        [ "$#" -ge 2 ] || fail "--desktop needs a directory"
        DESKTOP_DIR="$2"
        shift 2
        ;;
      --out)
        [ "$#" -ge 2 ] || fail "--out needs a directory"
        OUT_DIR="$2"
        shift 2
        ;;
      --check)
        CHECK=1
        shift
        ;;
      *) fail "unknown argument: $1" ;;
    esac
  done

  [ -n "$ENGINE_DIR" ] || fail "--engine <dir> is required"
  [ -n "$DESKTOP_DIR" ] || fail "--desktop <dir> is required"
  [ -n "$OUT_DIR" ] || fail "--out <dir> is required"
  [ -d "$ENGINE_DIR" ] || fail "no engine maintainer directory at $ENGINE_DIR"
  [ -d "$DESKTOP_DIR" ] || fail "no desktop script directory at $DESKTOP_DIR"
}

# One python process per script, because the whole job is exact bytes and a
# shell pipeline that reads and rewrites a file is where a trailing newline goes
# missing.
assemble_one() {
  local name="$1"
  python3 - "$ENGINE_DIR/$name" "$DESKTOP_DIR/$name" "$OUT_DIR/$name" "$CHECK" <<'PY'
import os
import sys

engine_path, desktop_path, out_path, check = sys.argv[1:5]
check = check == "1"

OPEN_ENGINE = "# >>> engine fragment, byte for byte from the engine artifact\n"
CLOSE_ENGINE = "# <<< engine fragment\n"
OPEN_DESKTOP = "# >>> desktop fragment, byte for byte from packaging/scripts\n"
CLOSE_DESKTOP = "# <<< desktop fragment\n"

PREAMBLE = """#!/bin/sh
# Assembled by scripts/assemble_maintainer.sh from two fragments. Do not edit:
# the engine half is the engine package's own script, byte for byte, and the
# build refuses any difference.
#
# Each fragment runs in a subshell, so a fragment's own `exit 0` ends that
# fragment rather than this script, and a fragment that fails stops the whole
# thing here through the `set -eu` below.
set -eu

"""

POSTAMBLE = "\nexit 0\n"


def refuse(sentence):
    sys.exit(f"assemble_maintainer: {sentence}")


def read(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return handle.read()
    except OSError as error:
        refuse(f"cannot read {path}: {error}")


def fragment(path, text):
    if not text:
        refuse(f"{path} is empty, and an empty maintainer fragment is not a step")
    if not text.endswith("\n"):
        refuse(f"{path} does not end with a newline, and the assembly relies on it")
    for marker in (OPEN_ENGINE, CLOSE_ENGINE, OPEN_DESKTOP, CLOSE_DESKTOP):
        if marker.strip() in text:
            refuse(f"{path} carries the assembly marker {marker.strip()!r}")
    return text


engine = fragment(engine_path, read(engine_path))
desktop = fragment(desktop_path, read(desktop_path))

assembled = (
    PREAMBLE
    + OPEN_ENGINE
    + "(\n"
    + engine
    + ")\n"
    + CLOSE_ENGINE
    + "\n"
    + OPEN_DESKTOP
    + "(\n"
    + desktop
    + ")\n"
    + CLOSE_DESKTOP
    + POSTAMBLE
)


def between(text, opening, closing, what):
    """The fragment's own bytes, read back out of the assembled script."""
    try:
        start = text.index(opening) + len(opening)
        end = text.index(closing, start)
    except ValueError:
        refuse(f"{out_path} carries no {what} fragment markers")
    body = text[start:end]
    if not body.startswith("(\n") or not body.endswith(")\n"):
        refuse(f"the {what} fragment in {out_path} is not wrapped in a subshell")
    return body[len("(\n"):-len(")\n")]


if check:
    if not os.path.isfile(out_path):
        refuse(f"no assembled script at {out_path}")
    current = read(out_path)
    for what, opening, closing, source, source_path in (
        ("engine", OPEN_ENGINE, CLOSE_ENGINE, engine, engine_path),
        ("desktop", OPEN_DESKTOP, CLOSE_DESKTOP, desktop, desktop_path),
    ):
        embedded = between(current, opening, closing, what)
        if embedded != source:
            refuse(
                f"the {what} half of {os.path.basename(out_path)} is not "
                f"{source_path} byte for byte"
            )
    if current != assembled:
        refuse(
            f"{out_path} is not what this assembly writes, although both "
            "fragments read back correctly; something outside the markers differs"
        )
    print(f"  {os.path.basename(out_path)}: both halves verbatim")
    sys.exit(0)

os.makedirs(os.path.dirname(out_path), exist_ok=True)
with open(out_path, "w", encoding="utf-8") as handle:
    handle.write(assembled)
os.chmod(out_path, 0o755)
print(f"  {os.path.basename(out_path)}: {len(engine)} engine bytes, {len(desktop)} desktop bytes")
PY
}

parse_arguments "$@"
mkdir -p "$OUT_DIR"

if [ "$CHECK" = "1" ]; then
  echo "assemble_maintainer: the assembled scripts against their fragments"
else
  echo "assemble_maintainer: the engine fragment, then the desktop fragment"
fi

for script in postinstall.sh postremove.sh; do
  assemble_one "$script"
done

# A maintainer script that does not parse is one dpkg finds out about on a
# user's machine. `sh -n` is the same reader that will run it.
if [ "$CHECK" != "1" ]; then
  for script in postinstall.sh postremove.sh; do
    sh -n "$OUT_DIR/$script" ||
      fail "the assembled $script does not parse as a POSIX shell script"
  done
  echo "  both assembled scripts parse"
fi
