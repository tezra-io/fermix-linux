#!/usr/bin/env python3
"""Build the app-engine archives scripts/verify_engine_test.sh verifies.

Two kinds, and they exist for two different reasons.

`honest` builds a tiny but structurally real archive: the root the engine
publishes, a staged tree with an executable, a symbolic link and a data file,
the maintainer fragments, the resolved nfpm contents, and an
`engine-manifest.json` whose `tree_sha256` is the engine's own canonical digest
over what the archive carries. One named break makes exactly one field of that
manifest wrong, so each refusal in verify_engine.sh is driven on its own.

`evil` builds an archive that is not a release tree at all: a traversal, a
device node, a hard link, a setuid mode. These are hand-assembled member by
member rather than tarred from a directory, because a directory on disk cannot
hold most of them.

The digest here is a second implementation of the record format in the engine's
scripts/release/package_app_engine.py, deliberately: a fixture that reused the
verifier's own code would prove only that the verifier agrees with itself.
"""

import gzip
import hashlib
import io
import os
import stat
import sys
import tarfile
import tempfile

ROOT = "fermix_app_engine"
MANIFEST_NAME = "engine-manifest.json"
ARCHITECTURES = {"linux_x86_64": "x86_64", "linux_aarch64": "aarch64"}
MANIFEST_BREAKS = (
    "none",
    "schema",
    "commit",
    "version",
    "distribution",
    "target",
    "arch",
    "identity",
    "digest",
    "nomanifest",
    "badjson",
    "manifestlink",
)


def die(sentence):
    sys.exit(f"make_engine_archive: {sentence}")


# ---- the honest archive ---------------------------------------------------


def write_file(root, relative, content, mode):
    path = os.path.join(root, relative)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(content)
    os.chmod(path, mode)


def build_tree(root, target):
    """A tree with one of everything the digest has a record for."""
    write_file(root, "tree/usr/bin/fermix", f"a fake engine for {target}\n", 0o755)
    write_file(root, "tree/usr/share/fermix/engine.json", '{"engine": "fake"}\n', 0o644)
    write_file(root, "maintainer/postinstall.sh", "#!/bin/sh\nexit 0\n", 0o755)
    write_file(root, "nfpm-contents.yaml", "contents:\n  - src: tree/usr/bin/fermix\n", 0o644)
    os.makedirs(os.path.join(root, "tree/usr/lib/fermix"), exist_ok=True)
    os.symlink("../../bin/fermix", os.path.join(root, "tree/usr/lib/fermix/current"))


def scan(root):
    entries = []
    for directory, subdirectories, files in os.walk(root):
        subdirectories.sort()
        for name in sorted(subdirectories) + sorted(files):
            path = os.path.join(directory, name)
            entries.append((os.path.relpath(path, root), path))
    entries.sort(key=lambda entry: entry[0])
    return entries


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def digest_record(relative, path):
    metadata = os.lstat(path)
    if stat.S_ISLNK(metadata.st_mode):
        return f"symlink\0{relative}\0{0o777:04o}\0{os.readlink(path)}\n".encode()
    mode = f"{stat.S_IMODE(metadata.st_mode):04o}"
    if stat.S_ISDIR(metadata.st_mode):
        return f"directory\0{relative}\0{mode}\n".encode()
    return f"file\0{relative}\0{mode}\0{file_sha256(path)}\n".encode()


def tree_digest(root):
    digest = hashlib.sha256()
    for relative, path in scan(root):
        if relative == MANIFEST_NAME:
            continue
        digest.update(digest_record(relative, path))
    return digest.hexdigest()


def manifest_for(root, target, version, commit, identity, build_id):
    return {
        "schema_version": 1,
        "identity": {
            "engine_id": "fermix-core",
            "product_version": version,
            "build_id": build_id,
            "source_commit": commit,
            "distribution_identity": "linux_package",
            "artifact_target": target,
            "architecture": ARCHITECTURES[target],
        },
        "protocols": {
            "management": {"current_version": 1, "minimum_version": 1, "maximum_version": 1},
            "realtime": {"current_version": 1, "minimum_version": 1, "maximum_version": 1},
        },
        "provenance": {
            "oidc_issuer": "https://token.actions.githubusercontent.com",
            "certificate_identity": identity,
        },
        "tree_sha256": tree_digest(root),
        "inventory": {"artifact_target": target, "architecture": ARCHITECTURES[target], "entries": []},
    }


def break_manifest(manifest, broken, target):
    """Make exactly one field wrong, and say so by name."""
    other = "linux_aarch64" if target == "linux_x86_64" else "linux_x86_64"
    if broken == "schema":
        manifest["schema_version"] = 2
    elif broken == "commit":
        manifest["identity"]["source_commit"] = "9" * 40
    elif broken == "version":
        manifest["identity"]["product_version"] = "9.9.9"
    elif broken == "distribution":
        manifest["identity"]["distribution_identity"] = "macos_app"
    elif broken == "target":
        manifest["identity"]["artifact_target"] = other
    elif broken == "arch":
        manifest["identity"]["architecture"] = "riscv64"
    elif broken == "identity":
        manifest["provenance"]["certificate_identity"] = "https://example.invalid/nobody"
    elif broken == "digest":
        manifest["tree_sha256"] = "0" * 64
    return manifest


def write_manifest(root, manifest, broken):
    import json

    path = os.path.join(root, MANIFEST_NAME)
    if broken == "nomanifest":
        return
    if broken == "badjson":
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("this is not JSON\n")
        return
    if broken == "manifestlink":
        os.symlink("nfpm-contents.yaml", path)
        return
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)
    os.chmod(path, 0o644)


def tar_tree(root, destination):
    """Archive the tree the way the engine's packager does: mode kept, no owner."""
    with open(destination, "wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.GNU_FORMAT) as archive:
                add_member(archive, archive.gettarinfo(root, arcname=ROOT), 0o755, None)
                for relative, path in scan(root):
                    info = archive.gettarinfo(path, arcname=f"{ROOT}/{relative}")
                    mode = 0o777 if info.issym() else stat.S_IMODE(os.lstat(path).st_mode)
                    add_member(archive, info, mode, path if info.isfile() else None)


def add_member(archive, info, mode, content_path):
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mtime = 0
    info.mode = mode
    if content_path is None:
        archive.addfile(info)
        return
    with open(content_path, "rb") as stream:
        archive.addfile(info, stream)


def honest(destination, target, version, commit, identity, build_id, broken):
    if target not in ARCHITECTURES:
        die(f"'{target}' is not a target the engine publishes")
    if broken not in MANIFEST_BREAKS:
        die(f"'{broken}' is not a manifest break this fixture knows")
    with tempfile.TemporaryDirectory(prefix="engine-fixture-") as work:
        root = os.path.join(work, ROOT)
        os.mkdir(root, 0o755)
        build_tree(root, target)
        manifest = manifest_for(root, target, version, commit, identity, build_id)
        write_manifest(root, break_manifest(manifest, broken, target), broken)
        tar_tree(root, destination)


# ---- the archives a release tree could never be ---------------------------


def entry(name, kind, mode=0o644, link=""):
    info = tarfile.TarInfo(name)
    info.type = kind
    info.mode = mode
    info.mtime = 0
    info.linkname = link
    return info


def put(archive, info, data=b""):
    info.size = len(data)
    archive.addfile(info, io.BytesIO(data) if data else None)


def evil_members(archive, kind):
    """Each kind is one refusal in verify_engine.sh, in archive form."""
    root = entry(ROOT, tarfile.DIRTYPE, 0o755)
    if kind != "root":
        put(archive, root)
    if kind == "root":
        put(archive, entry("other_root", tarfile.DIRTYPE, 0o755))
        put(archive, entry("other_root/engine", tarfile.REGTYPE), b"elsewhere\n")
    elif kind == "absolute":
        put(archive, entry("/etc/passwd", tarfile.REGTYPE), b"root:x:0:0\n")
    elif kind == "dotdot":
        put(archive, entry(f"{ROOT}/../escape", tarfile.REGTYPE), b"escaped\n")
    elif kind == "duplicate":
        put(archive, entry(f"{ROOT}/twice", tarfile.REGTYPE), b"first\n")
        put(archive, entry(f"{ROOT}/twice", tarfile.REGTYPE), b"second\n")
    elif kind == "undersymlink":
        put(archive, entry(f"{ROOT}/tree", tarfile.SYMTYPE, 0o777, "."))
        put(archive, entry(f"{ROOT}/tree/planted", tarfile.REGTYPE), b"planted\n")
    elif kind == "symlinkescape":
        put(archive, entry(f"{ROOT}/out", tarfile.SYMTYPE, 0o777, "../../etc/passwd"))
    elif kind == "implicitdir":
        put(archive, entry(f"{ROOT}/tree/usr/bin/fermix", tarfile.REGTYPE, 0o755), b"no parents\n")
    elif kind == "device":
        node = entry(f"{ROOT}/node", tarfile.CHRTYPE, 0o666)
        node.devmajor, node.devminor = 1, 3
        put(archive, node)
    elif kind == "hardlink":
        put(archive, entry(f"{ROOT}/original", tarfile.REGTYPE), b"original\n")
        put(archive, entry(f"{ROOT}/linked", tarfile.LNKTYPE, 0o644, f"{ROOT}/original"))
    elif kind == "setuid":
        put(archive, entry(f"{ROOT}/suid", tarfile.REGTYPE, 0o4755), b"setuid\n")
    elif kind == "setgid":
        put(archive, entry(f"{ROOT}/sgid", tarfile.REGTYPE, 0o2755), b"setgid\n")
    elif kind == "deep":
        name = ROOT
        for level in range(40):
            name = f"{name}/d{level}"
            put(archive, entry(name, tarfile.DIRTYPE, 0o755))
    else:
        die(f"'{kind}' is not an archive shape this fixture knows")


def evil(destination, kind):
    if kind == "notgzip":
        with open(destination, "wb") as handle:
            handle.write(b"this is not a gzip stream, let alone a tar\n")
        return
    with open(destination, "wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.GNU_FORMAT) as archive:
                evil_members(archive, kind)


def main(argv):
    if len(argv) < 3:
        die("usage: make_engine_archive.py honest|evil <archive> ...")
    command, destination = argv[1], argv[2]
    if command == "honest":
        if len(argv) != 9:
            die("usage: make_engine_archive.py honest <archive> <target> <version> "
                "<commit> <identity> <build-id> <break>")
        honest(destination, *argv[3:9])
        return 0
    if command == "evil":
        if len(argv) != 4:
            die("usage: make_engine_archive.py evil <archive> <kind>")
        evil(destination, argv[3])
        return 0
    die(f"'{command}' is not a command this fixture knows")


if __name__ == "__main__":
    sys.exit(main(sys.argv))
