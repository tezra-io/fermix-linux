#!/usr/bin/env bash
#
# The private toolkit runtime in the staged tree is wired the way the design
# says it is (amendment section 8.2).
#
# Four claims, each of which is invisible until a user's machine disagrees with
# it, and each of which is a one-line mistake to make:
#
#   1. **Every object finds its libraries through its own `RUNPATH`.** Not
#      through `LD_LIBRARY_PATH`, which every child would inherit, and not
#      through an `RPATH`, which the loader consults before `LD_LIBRARY_PATH`
#      and which cannot be overridden by a person exercising LGPL 2.1 section 6.
#      What is checked is where the `RUNPATH` leads from where the object sits,
#      not how it is spelled: an object in `lib/cairo` may say `$ORIGIN/..` or
#      `$ORIGIN/../../lib`, and those are one directory. An entry that does not
#      resolve to the private `lib/` is refused by name.
#   2. **Nothing reaches out of the prefix except to the host boundary.** Every
#      `NEEDED` entry either names a file inside the prefix or is on the host
#      list in `packaging/runtime/RUNTIME.lock.json`, which is section 4.1's
#      host column written down. A private object naming a host GTK, GLib,
#      pango or cairo is called out in its own sentence, because that is the
#      failure this whole design exists to prevent and "not on the host list"
#      would under-describe it.
#   3. **The generated caches name only private paths.** `loaders.cache` and
#      the compiled schemas are generated at build time against the final
#      absolute prefix, which is what lets the application set no
#      `GDK_PIXBUF_MODULE_FILE` and no `GIO_MODULE_DIR`. A cache with a build
#      path or a host path in it is a window with no images and no settings.
#   4. **The engine's own executables are not held to any of this.** They are
#      static, or they name a musl loader under `/var/lib/fermix/runtimes`, and
#      either way they link nothing from the host and carry no `RUNPATH`. They
#      are identified by what they are rather than by where they sit, and an
#      engine object that turned out to name a glibc interpreter is a refusal.
#
# Usage:
#   check_private_runtime.sh <stage>
#   check_private_runtime.sh <stage> --lock <RUNTIME.lock.json>
#   check_private_runtime.sh <stage> --engine-tree <dir>
#
# --engine-tree names another tree the package is built from. The engine's files
# are read straight out of the verified staging directory rather than copied
# into the stage, and claim 4 above is about objects that live there, so the
# gate is told where they are rather than concluding they are absent.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PREFIX="/usr/lib/fermix-desktop"
STAGE=""
LOCK="$ROOT_DIR/packaging/runtime/RUNTIME.lock.json"
ENGINE_TREES=()

fail() {
  echo "check_private_runtime: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --lock)
      [ "$#" -ge 2 ] || fail "--lock needs a path"
      LOCK="$2"
      shift 2
      ;;
    --engine-tree)
      [ "$#" -ge 2 ] || fail "--engine-tree needs a directory"
      [ -d "$2" ] || fail "no engine tree at $2"
      ENGINE_TREES+=("$2")
      shift 2
      ;;
    --*) fail "unknown argument: $1" ;;
    *)
      [ -z "$STAGE" ] || fail "unexpected argument: $1"
      STAGE="$1"
      shift
      ;;
  esac
done

[ -n "$STAGE" ] || fail "usage: check_private_runtime.sh <stage> [--lock <path>]"
[ -d "$STAGE" ] || fail "no staged tree at $STAGE"
[ -f "$LOCK" ] || fail "no runtime lock file at $LOCK"
[ -d "$STAGE$PREFIX" ] ||
  fail "the staged tree carries no private prefix at $STAGE$PREFIX"
command -v python3 >/dev/null 2>&1 || fail "python3 is not installed"

echo "check_private_runtime: the staged tree at $STAGE"

python3 - "$STAGE" "$PREFIX" "$LOCK" "$ROOT_DIR/scripts" \
  ${ENGINE_TREES[0]+"${ENGINE_TREES[@]}"} <<'PY' || exit 1
import json
import os
import re
import sys

stage, prefix, lock_path, scripts_dir = sys.argv[1:5]
engine_trees = sys.argv[5:]
sys.path.insert(0, scripts_dir)
import elf_facts  # noqa: E402  (the path is set one line above)

PRIVATE_ROOT = os.path.join(stage, prefix.lstrip("/"))
LIB = os.path.join(PRIVATE_ROOT, "lib")
ENGINE_LOADER_STORE = "/var/lib/fermix/runtimes/"

# The sonames whose presence on the host side would mean two copies of one
# toolkit in one process. Named rather than left to the host list, so the
# refusal says what actually went wrong.
TOOLKIT = (
    "libgtk-4.so",
    "libgtk-3.so",
    "libadwaita-1.so",
    "libglib-2.0.so",
    "libgobject-2.0.so",
    "libgio-2.0.so",
    "libgmodule-2.0.so",
    "libpango-1.0.so",
    "libpangocairo-1.0.so",
    "libpangoft2-1.0.so",
    "libcairo.so",
    "libcairo-gobject.so",
    "libgdk_pixbuf-2.0.so",
)

problems = []


def refuse(sentence):
    problems.append(sentence)


with open(lock_path, encoding="utf-8") as handle:
    lock = json.load(handle)
host_libraries = set(lock.get("host_libraries") or ())
if not host_libraries:
    refuse(f"{lock_path} carries no host_libraries list, and it is the host boundary")

# ---------------------------------------------------------------- the prefix

if not os.path.isdir(LIB):
    refuse(f"the private prefix carries no lib directory at {LIB}")

# Every name the private prefix can answer a NEEDED entry with, links included:
# a NEEDED names a SONAME, and a SONAME is usually the link rather than the
# file it points at.
carried = set()
for directory, subdirectories, names in os.walk(PRIVATE_ROOT):
    subdirectories.sort()
    for name in names:
        carried.add(name)


def expected_runpath(path):
    """The plainest spelling of this object's RUNPATH, for the refusal text."""
    relative = os.path.relpath(LIB, os.path.dirname(path))
    return "$ORIGIN" if relative == "." else os.path.join("$ORIGIN", relative)


def runpath_reaches_lib(runpath, path):
    """Does this RUNPATH lead to the private lib directory, however it is spelled?

    The question is where the loader ends up, not how the toolkit's build system
    wrote it down. An object in lib/cairo can say `$ORIGIN/..` or
    `$ORIGIN/../../lib`, and both are the same directory; demanding one spelling
    refuses a tree that is correct, which is worse than not asking at all.
    Several colon-separated entries are allowed, and one of them has to be it.
    """
    if not runpath:
        return False
    origin = os.path.dirname(os.path.abspath(path))
    target = os.path.realpath(LIB)
    for entry in runpath.split(":"):
        entry = entry.strip()
        if not entry:
            continue
        # $ORIGIN is the only variable the design allows, and an entry without
        # it is an absolute path this tree cannot promise anything about.
        if "$ORIGIN" not in entry and "${ORIGIN}" not in entry:
            continue
        resolved = entry.replace("${ORIGIN}", origin).replace("$ORIGIN", origin)
        if os.path.realpath(resolved) == target:
            return True
    return False


def is_engine_object(facts):
    """An engine executable: static, or launched by the engine's own loader."""
    interpreter = facts["interpreter"]
    if interpreter is not None:
        return interpreter.startswith(ENGINE_LOADER_STORE)
    return not facts["dynamic"] or not facts["needed"]


private = []
engine = []
for tree in [stage, *engine_trees]:
    for path, facts in elf_facts.walk(tree):
        inside = os.path.abspath(path).startswith(os.path.abspath(PRIVATE_ROOT) + os.sep)
        if inside:
            private.append((path, facts))
        elif is_engine_object(facts):
            engine.append((path, facts))
        else:
            refuse(
                f"{os.path.relpath(path, tree)} is an ELF outside the private prefix "
                f"that names the interpreter {facts['interpreter']!r}; every ELF in "
                "this package is either the private runtime or the engine's own"
            )

if not private:
    refuse(f"there is no ELF at all under {PRIVATE_ROOT}")

# ------------------------------------------------- 1. the search path, per object

for path, facts in private:
    shown = os.path.relpath(path, stage)
    wanted = expected_runpath(path)
    if facts["rpath"] is not None:
        refuse(
            f"{shown} carries an RPATH ({facts['rpath']}), and the design links "
            "with --enable-new-dtags so that a person can override it"
        )
    if not runpath_reaches_lib(facts["runpath"], path):
        refuse(
            f"{shown} has RUNPATH {facts['runpath']!r}, and from where it sits that "
            f"does not lead to the private lib directory; {wanted!r} would"
        )

# --------------------------------------------- 2. nothing reaches past the host

for path, facts in private:
    shown = os.path.relpath(path, stage)
    for soname in facts["needed"]:
        if soname in carried:
            continue
        if any(soname.startswith(toolkit) for toolkit in TOOLKIT):
            refuse(
                f"{shown} needs {soname} and the private prefix does not carry it, so "
                "it would load the host's copy and put two toolkits in one process"
            )
            continue
        if soname not in host_libraries:
            refuse(
                f"{shown} needs {soname}, which is neither carried in the private "
                f"prefix nor on the host list in {os.path.basename(lock_path)}"
            )

# ------------------------------------------------- 3. the generated caches

# A filesystem path, as opposed to the other things in these files that begin
# with a slash. A compiled GSettings schema is full of object paths like
# `/org/gtk/gtk4/settings/debug/`, and the pixbuf cache carries mime patterns
# like `/*`; neither is a place on disk, and reading them as one turns a correct
# tree into a wall of refusals. The roots below are the ones a Linux filesystem
# actually has, so a path outside the prefix is still caught.
FILESYSTEM_PATH = re.compile(
    r"^/(usr|etc|opt|var|lib|lib64|bin|sbin|home|root|srv|tmp|run)(/|$)"
)


def check_generated(path, what, must_name_a_file=False):
    if not os.path.isfile(path):
        refuse(f"there is no {what} at {os.path.relpath(path, stage)}")
        return
    with open(path, "rb") as handle:
        body = handle.read()

    named_inside = 0
    for piece in body.split(b"\x00"):
        for candidate in piece.split():
            # The quotes come off first: the pixbuf cache writes each module's
            # path quoted, and a check for a leading slash on the quote would
            # look at every line and see nothing.
            text = candidate.decode("utf-8", "replace").strip('"')
            if not FILESYSTEM_PATH.match(text):
                continue
            if text.startswith(prefix + "/"):
                named_inside += 1
                continue
            refuse(
                f"{what} names {text}, which is outside the private prefix; the "
                "caches are generated against the final absolute prefix so that the "
                "application needs no environment variable to find them"
            )

    # A cache that names no path at all would pass the loop above by saying
    # nothing, and a pixbuf cache with no module in it is not a cache.
    if must_name_a_file and named_inside == 0:
        refuse(
            f"{what} names no file inside the private prefix, so either it is empty "
            "or it was generated against a prefix this package does not install"
        )


check_generated(
    os.path.join(LIB, "gdk-pixbuf-2.0", "2.10.0", "loaders.cache"),
    "the pixbuf loader cache",
    must_name_a_file=True,
)
check_generated(
    os.path.join(PRIVATE_ROOT, "share", "glib-2.0", "schemas", "gschemas.compiled"),
    "the compiled GSettings schemas",
)

# --------------------------------------------------- 4. the application itself

application = os.path.join(PRIVATE_ROOT, "bin", "fermix-desktop")
if not os.path.isfile(application):
    refuse(f"there is no application ELF at {prefix}/bin/fermix-desktop")
else:
    facts = elf_facts.read(application)
    if facts is None:
        refuse(f"{prefix}/bin/fermix-desktop is not an ELF file")
    elif facts["runpath"] != "$ORIGIN/../lib":
        refuse(
            f"the application ELF has RUNPATH {facts['runpath']!r}, and it must be "
            "'$ORIGIN/../lib' so that it needs no LD_LIBRARY_PATH at all"
        )

# ------------------------------------------------------------------- the answer

for problem in problems:
    print(f"check_private_runtime: {problem}", file=sys.stderr)
if problems:
    sys.exit(1)

print(f"  {len(private)} private objects, every RUNPATH computed from where it sits")
print(f"  {len(engine)} engine objects, static or launched by the engine's own loader")
print("  the pixbuf cache and the compiled schemas name only private paths")
PY

echo "check_private_runtime: the window finds its toolkit inside the package"
