#!/usr/bin/env python3
"""Hold every meson flag in the lock file to the option upstream declares.

A wrong flag is otherwise found by `meson setup` when the build reaches that
component, up to an hour in. The same answer is in the component's own
meson_options.txt or meson.options, inside the tarball the lock file pins, so
this reads it there.

It needs the source cache. Components whose tarball is not cached are skipped
and counted, and the run says how many it checked.
"""

import argparse
import json
import os
import re
import sys
import tarfile

OPTION_FILES = ("meson_options.txt", "meson.options")
VALUES = {
    "boolean": {"true", "false"},
    "feature": {"enabled", "disabled", "auto"},
}
# Meson's own options, which no project declares. Only the ones the lock uses.
BUILTINS = {"auto_features": VALUES["feature"]}


def option_blocks(text):
    """Yield the body of each option(...) call, brackets balanced."""
    text = re.sub(r"#[^\n]*", "", text)
    index = 0
    while True:
        index = text.find("option(", index)
        if index < 0:
            return
        cursor = index + len("option(")
        depth = 1
        while cursor < len(text) and depth:
            if text[cursor] in "([":
                depth += 1
            elif text[cursor] in ")]":
                depth -= 1
            cursor += 1
        yield text[index + len("option("): cursor - 1]
        index = cursor


def parse_options(text):
    declared = {}
    for body in option_blocks(text):
        name = re.match(r"\s*'([^']+)'", body)
        if not name:
            continue
        kind = re.search(r"type\s*:\s*'(\w+)'", body)
        choices = re.search(r"choices\s*:\s*\[(.*?)\]", body, re.S)
        # `deprecated: {'true': 'enabled'}` still accepts the old spelling with a
        # warning; the lock carries the spelling upstream now prefers.
        legacy = re.search(r"deprecated\s*:\s*\{(.*?)\}", body, re.S)
        # `deprecated: 'other'` or `deprecated: true` retires the whole option.
        retired = re.search(r"deprecated\s*:\s*('[^']*'|true)", body)
        declared[name.group(1)] = (
            kind.group(1) if kind else "unknown",
            set(re.findall(r"'([^']+)'", choices.group(1))) if choices else None,
            set(re.findall(r"'([^']+)'\s*:", legacy.group(1))) if legacy else set(),
            retired.group(1) if retired else None,
        )
    return declared


def read_option_file(archive, source_dir):
    """The component's option file, read straight out of its tarball."""
    with tarfile.open(archive) as tar:
        for name in OPTION_FILES:
            for candidate in ("%s/%s" % (source_dir, name),
                              "./%s/%s" % (source_dir, name)):
                try:
                    handle = tar.extractfile(candidate)
                except KeyError:
                    continue
                if handle is not None:
                    return handle.read().decode("utf-8", "replace")
    return None


def check_flag(name, key, value, declared):
    if key in BUILTINS:
        if value not in BUILTINS[key]:
            return "%s: %s=%s is not a value meson takes" % (name, key, value)
        return None
    if key not in declared:
        return "%s: %s is not an option upstream declares" % (name, key)
    kind, choices, legacy, retired = declared[key]
    if retired:
        return "%s: %s is deprecated upstream (%s)" % (name, key, retired)
    allowed = set(choices) if choices else set(VALUES.get(kind, ()))
    if allowed and value in legacy:
        return "%s: %s=%s is the deprecated spelling; upstream takes %s" % (
            name, key, value, ", ".join(sorted(allowed)))
    if allowed and value not in allowed:
        return "%s: %s=%s, but it is a %s taking %s" % (
            name, key, value, kind, ", ".join(sorted(allowed)))
    return None


def check_component(component, cache):
    archive = os.path.join(cache, component["archive"])
    text = read_option_file(archive, component["source_dir"])
    if text is None and not component["options"]:
        return []
    if text is None:
        return ["%s: flags given, and no option file in %s"
                % (component["name"], component["archive"])]
    declared = parse_options(text)
    problems = []
    for flag in component["options"]:
        if not flag.startswith("-D"):
            problems.append("%s: %s is not a -D flag" % (component["name"], flag))
            continue
        key, _, value = flag[2:].partition("=")
        problem = check_flag(component["name"], key, value, declared)
        if problem:
            problems.append(problem)
    return problems


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", required=True)
    parser.add_argument("--sources", required=True)
    args = parser.parse_args(argv)

    with open(args.lock, encoding="utf-8") as handle:
        lock = json.load(handle)

    meson = [c for c in lock["components"] if c["build_system"] == "meson"]
    cached = [c for c in meson
              if os.path.exists(os.path.join(args.sources, c["archive"]))]
    problems = []
    for component in cached:
        problems.extend(check_component(component, args.sources))
    for line in problems:
        print("check_options: %s" % line, file=sys.stderr)
    if problems:
        return 1
    if not cached:
        print("  skipped: none of the %d meson components is in %s"
              % (len(meson), args.sources))
        return 0
    print("  ok: every meson flag is an option upstream declares"
          " (%d of %d components cached)" % (len(cached), len(meson)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
