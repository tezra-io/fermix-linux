#!/usr/bin/env python3
"""Hold every meson flag in the lock file to the option upstream actually declares.

A wrong flag is not caught until `meson setup` reaches that component, which on
this build is up to an hour in, and the message it produces ("Value \"false\" for
option \"lzma\" is not one of the choices") is one nobody sees until then. The
same answer is in the component's own meson_options.txt, which is inside the
tarball the lock file already pins, so this reads it there instead.

It needs the source cache, so it is a gate that runs when the tarballs are
already downloaded and says so and passes when they are not. That is deliberate:
a check that forces a fresh clone to download three hundred megabytes is a check
people turn off.
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
        yield text[index + len("option(") : cursor - 1]
        index = cursor


def parse_options(text):
    declared = {}
    for body in option_blocks(text):
        name = re.match(r"\s*'([^']+)'", body)
        if not name:
            continue
        kind = re.search(r"type\s*:\s*'(\w+)'", body)
        choices = re.search(r"choices\s*:\s*\[(.*?)\]", body, re.S)
        # A `deprecated : { 'true': 'enabled' }` mapping means the old spelling
        # is still accepted, with a warning. It is not an error, and a check that
        # called it one would be wrong; the lock file should still carry the
        # spelling upstream now prefers, which is what the mapping's values are.
        legacy = re.search(r"deprecated\s*:\s*\{(.*?)\}", body, re.S)
        declared[name.group(1)] = (
            kind.group(1) if kind else "unknown",
            set(re.findall(r"'([^']+)'", choices.group(1))) if choices else None,
            set(re.findall(r"'([^']+)'\s*:", legacy.group(1))) if legacy else set(),
        )
    return declared


def read_option_file(archive, source_dir):
    """The component's option file, read straight out of its tarball."""
    with tarfile.open(archive) as tar:
        for name in OPTION_FILES:
            for candidate in ("%s/%s" % (source_dir, name), "./%s/%s" % (source_dir, name)):
                try:
                    handle = tar.extractfile(candidate)
                except KeyError:
                    continue
                if handle is not None:
                    return handle.read().decode("utf-8", "replace")
    return None


def check_component(component, cache):
    archive = os.path.join(cache, component["archive"])
    if component["build_system"] != "meson" or not os.path.exists(archive):
        return []
    text = read_option_file(archive, component["source_dir"])
    if text is None:
        return []

    declared = parse_options(text)
    problems = []
    for flag in component["options"]:
        if not flag.startswith("-D"):
            continue
        key, _, value = flag[2:].partition("=")
        if key not in declared:
            problems.append("%s: %s is not an option upstream declares" % (component["name"], key))
            continue
        kind, choices, legacy = declared[key]
        allowed = set(choices) if choices else set(VALUES.get(kind, ()))
        if allowed and value in legacy:
            problems.append(
                "%s: %s=%s is the deprecated spelling; upstream now takes %s"
                % (component["name"], key, value, ", ".join(sorted(allowed)))
            )
        elif allowed and value not in allowed:
            problems.append(
                "%s: %s=%s, but it is a %s taking %s"
                % (component["name"], key, value, kind, ", ".join(sorted(allowed)))
            )
    return problems


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", required=True)
    parser.add_argument("--sources", required=True)
    args = parser.parse_args(argv)

    with open(args.lock, encoding="utf-8") as handle:
        lock = json.load(handle)

    checked, problems = 0, []
    for component in lock["components"]:
        if component["build_system"] != "meson":
            continue
        if not os.path.exists(os.path.join(args.sources, component["archive"])):
            continue
        checked += 1
        problems.extend(check_component(component, args.sources))

    if not checked:
        print("  skipped: no cached sources to read upstream's options from")
        return 0
    for line in problems:
        print("check_options: %s" % line, file=sys.stderr)
    if problems:
        return 1
    print("  ok: every meson flag matches the option upstream declares (%d components)" % checked)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
