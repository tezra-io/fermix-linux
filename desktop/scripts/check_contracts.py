#!/usr/bin/env python3
"""The release check on the vendored wire contracts: each is the pinned engine's, byte for byte.

Each directory under desktop/core/contracts/ (management, companion, realtime) is a contract copied
from the engine: a CHECKSUMS.txt listing its files, and a SOURCE.md naming the commit they were
copied from and each file's engine path. A release is refused unless, for every one of them:

- it has a SOURCE.md, whose commit is the commit desktop/engine/PIN.json pins (`source_commit`);
- every file SOURCE.md names is one CHECKSUMS.txt lists; and
- every file CHECKSUMS.txt lists is byte for byte the engine's file at that commit (the same
  SHA-256), read through gh, whose token must be able to read the pin's repository.

The contract tests hold the client's protocol version to the vendored files (companion and realtime
to the schema's version and window, management to the golden request frames), so byte identity is
what keeps an app speaking companion 4 from being packaged with an engine serving 2 to 3, where
every chat would be refused (M60 cues §7). Every contract is checked and every refusal printed
before the check fails.

    check_contracts.py [--pin <file>] [--contracts <dir>]

--pin defaults to desktop/engine/PIN.json, which engine_pin.py checks whole first; --contracts to
desktop/core/contracts. Reading the engine needs gh, which reads GH_TOKEN.
"""

import argparse
import hashlib
import os
import re
import subprocess
import sys
import time

DESKTOP = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_PIN = os.path.join(DESKTOP, "engine", "PIN.json")
DEFAULT_CONTRACTS = os.path.join(DESKTOP, "core", "contracts")
ENGINE_PIN = os.path.join(DESKTOP, "scripts", "engine_pin.py")
COMMIT_LINE = re.compile(r"^- Commit: `([0-9a-f]{40})`", re.MULTILINE)
PATH_ROW = re.compile(r"^\| `([^`]+)` \| `([^`]+)` \|$", re.MULTILINE)
CHECKSUM_LINE = re.compile(r"[0-9a-f]{64}  (\S.*)")
# A read of the engine is tried this many times, a pause apart, and each attempt is cut off after
# GH_TIMEOUT_SECONDS; then the file is refused, naming it.
GH_ATTEMPTS = 3
GH_PAUSE_SECONDS = 2
GH_TIMEOUT_SECONDS = 120


class Refusal(Exception):
    """A refusal of the whole check; its message is printed after "check_contracts: "."""


def pinned(pin_path, field):
    """One field of the pin, which engine_pin.py refuses unless the whole pin is complete."""
    result = subprocess.run(
        [sys.executable, "-I", ENGINE_PIN, "--pin", pin_path, "--get", field],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise Refusal(f"the engine pin {pin_path} is refused: {result.stderr.strip()}")
    return result.stdout.strip()


def contracts(root):
    """The vendored contracts under `root`, by name: every directory there."""
    try:
        names = sorted(os.listdir(root))
    except OSError as error:
        raise Refusal(f"cannot read {root}: {error}") from error
    found = [n for n in names if os.path.isdir(os.path.join(root, n))]
    if not found:
        raise Refusal(f"no vendored contracts under {root}")
    return found


def read_text(path):
    with open(path, encoding="utf-8") as stream:
        return stream.read()


def read_bytes(path):
    """The file's bytes, or None, saying why on stderr."""
    try:
        with open(path, "rb") as stream:
            return stream.read()
    except OSError as error:
        print(f"check_contracts: {path}: {error}", file=sys.stderr)
        return None


def listed(directory):
    """The files CHECKSUMS.txt lists, in its order; None unless it is sha256sum's format."""
    text = read_bytes(os.path.join(directory, "CHECKSUMS.txt"))
    if text is None:
        return None
    lines = [CHECKSUM_LINE.fullmatch(line) for line in text.decode("utf-8", "replace").splitlines()]
    if not lines or None in lines:
        return None
    return [line.group(1) for line in lines]


def gh_once(request):
    """One read of `request` through gh: the bytes it printed, or None, saying why on stderr."""
    try:
        result = subprocess.run(
            ["gh", "api", "-H", "Accept: application/vnd.github.raw+json", request],
            check=False,
            capture_output=True,
            timeout=GH_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired:
        print(f"check_contracts: gh did not answer {request} in time", file=sys.stderr)
        return None
    if result.returncode != 0:
        why = result.stderr.decode("utf-8", "replace").strip()
        print(f"check_contracts: {request}: {why}", file=sys.stderr)
        return None
    return result.stdout


def engine_file(repository, commit, path):
    """The bytes of the engine's file at `path` and `commit`; None when no attempt could read it."""
    request = f"repos/{repository}/contents/{path}?ref={commit}"
    for attempt in range(1, GH_ATTEMPTS + 1):
        answer = gh_once(request)
        if answer is not None:
            return answer
        if attempt < GH_ATTEMPTS:
            time.sleep(GH_PAUSE_SECONDS)
    return None


def check_file(name, directory, file, engine_path, pin):
    """The refusal for one vendored file, or None when it is the pinned engine's, byte for byte."""
    if engine_path is None:
        return f"{name}: SOURCE.md names no engine path for {file}"
    vendored = read_bytes(os.path.join(directory, file))
    if vendored is None:
        return f"{name}: {file} is listed in CHECKSUMS.txt and cannot be read here"
    served = engine_file(pin["repository"], pin["commit"], engine_path)
    if served is None:
        return (
            f"{name}: the pinned engine's {file} could not be read from {pin['repository']} "
            f"at {pin['commit']}; the token gh uses (GH_TOKEN) must be able to read "
            f"{pin['repository']}"
        )
    here, there = hashlib.sha256(vendored).hexdigest(), hashlib.sha256(served).hexdigest()
    if here != there:
        return (
            f"{name}: {file} is not the pinned engine's {engine_path} "
            f"(sha256 here {here[:12]}, at the pin {there[:12]})"
        )
    return None


def check_one(name, root, pin):
    """Every refusal for the contract `name`, as sentences; none when it holds."""
    directory = os.path.join(root, name)
    try:
        source = read_text(os.path.join(directory, "SOURCE.md"))
    except OSError as error:
        return [f"{name}: SOURCE.md cannot be read: {error.strerror}"]
    named = COMMIT_LINE.search(source)
    if named is None:
        return [f"{name}: SOURCE.md names no commit"]
    refusals = []
    if named.group(1) != pin["commit"]:
        refusals.append(
            f"{name}: SOURCE.md names {named.group(1)}, and the pin names {pin['commit']}"
        )
    files = listed(directory)
    if files is None:
        return refusals + [f"{name}: CHECKSUMS.txt is missing, empty or not sha256sum's format"]
    paths = dict(PATH_ROW.findall(source))
    refusals.extend(
        f"{name}: SOURCE.md names {file}, which CHECKSUMS.txt does not list"
        for file in sorted(set(paths) - set(files))
    )
    for file in files:
        refusal = check_file(name, directory, file, paths.get(file), pin)
        if refusal is not None:
            refusals.append(refusal)
    return refusals


def main(argv):
    parser = argparse.ArgumentParser(prog="check_contracts.py")
    parser.add_argument("--pin", default=DEFAULT_PIN)
    parser.add_argument("--contracts", default=DEFAULT_CONTRACTS)
    arguments = parser.parse_args(argv)
    try:
        pin = {
            "commit": pinned(arguments.pin, "source_commit"),
            "repository": pinned(arguments.pin, "repository"),
        }
        names = contracts(arguments.contracts)
    except Refusal as refusal:
        print(f"check_contracts: {refusal}", file=sys.stderr)
        return 1
    refusals = []
    for name in names:
        refusals.extend(check_one(name, arguments.contracts, pin))
    for refusal in refusals:
        print(f"check_contracts: {refusal}", file=sys.stderr)
    if refusals:
        return 1
    print(f"check_contracts: {', '.join(names)} are engine {pin['commit']}'s, byte for byte")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
