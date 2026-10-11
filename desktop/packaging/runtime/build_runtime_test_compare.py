#!/usr/bin/env python3
"""compare_manifest.py's own tests, run by build_runtime_test.sh.

A reference manifest and copies of it, each changed in one way --verify has to
catch: the lock, the architecture, an archive's digest, an archive or a file
gained or lost, a file's digest and a link's target.

Usage: build_runtime_test_compare.py <compare_manifest.py> <work dir>
"""

import copy
import json
import os
import subprocess
import sys

REFERENCE = {
    "lock": {"schema_version": 1},
    "architecture": "amd64",
    "archives": {"runtime-amd64.tar": "a" * 64, "runtime-licenses.json": "b" * 64},
    "files": [
        {"path": "lib/libone.so.1", "kind": "link", "target": "libone.so.1.0"},
        {"path": "lib/libone.so.1.0", "kind": "file", "sha256": "c" * 64},
        {"path": "share/data.txt", "kind": "file", "sha256": "d" * 64},
    ],
}


def fail(message):
    raise SystemExit("compare_test: " + message)


def changed(change):
    fresh = copy.deepcopy(REFERENCE)
    change(fresh)
    return fresh


CASES = [
    ("the lock file recorded in the manifest is not the one rebuilt",
     lambda m: m["lock"].update(schema_version=2)),
    ("architecture amd64 was rebuilt as arm64", lambda m: m.update(architecture="arm64")),
    ("archive differs: runtime-licenses.json",
     lambda m: m["archives"].update({"runtime-licenses.json": "e" * 64})),
    ("new archive in the rebuild: runtime-crates.tar.gz",
     lambda m: m["archives"].update({"runtime-crates.tar.gz": "f" * 64})),
    ("missing from the rebuild: runtime-amd64.tar",
     lambda m: m["archives"].pop("runtime-amd64.tar")),
    ("missing from the rebuild: share/data.txt", lambda m: m["files"].pop()),
    ("new in the rebuild: share/more.txt",
     lambda m: m["files"].append({"path": "share/more.txt", "kind": "file", "sha256": "0" * 64})),
    ("differs: lib/libone.so.1.0", lambda m: m["files"][1].update(sha256="1" * 64)),
    ("differs: lib/libone.so.1 (-> libone.so.1.0 -> -> libone.so.2)",
     lambda m: m["files"][0].update(target="libone.so.2")),
]


def run(script, work, label, fresh):
    reference_path = os.path.join(work, "reference.json")
    fresh_path = os.path.join(work, label + ".json")
    for path, manifest in ((reference_path, REFERENCE), (fresh_path, fresh)):
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(manifest, handle)
    return subprocess.run([sys.executable, script, reference_path, fresh_path],
                          capture_output=True, text=True, check=False)


def main(argv):
    if len(argv) != 2:
        fail("usage: build_runtime_test_compare.py <compare_manifest.py> <work dir>")
    script, work = os.path.abspath(argv[0]), os.path.abspath(argv[1])
    os.makedirs(work, exist_ok=True)
    same = run(script, work, "same", copy.deepcopy(REFERENCE))
    if same.returncode != 0 or "the rebuild is identical" not in same.stdout:
        fail("an identical manifest was not accepted: " + same.stderr)
    for number, (reason, change) in enumerate(CASES):
        result = run(script, work, "case-%d" % number, changed(change))
        if result.returncode == 0 or reason not in result.stderr:
            fail("not refused with '%s': %s" % (reason, result.stderr))
    print("  ok: an identical rebuild passes; a changed lock, architecture, archive,"
          " file or link each fails, by name")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
