#!/usr/bin/env python3
"""Verifies the pinned engine packages of one architecture, and stages what fermix-desktop carries
of them. Only this program writes the stage, and only once every check has passed.

    verify_engine.py [--pin <file>] <download dir> <stage dir> <arch>
    verify_engine.py --dev <stage dir> <local .deb> [<local .rpm>]

<download dir> is what fetch_engine.sh left. <arch> is amd64 or arm64 (x86_64 and aarch64 name the
same). The deb, then the rpm, of that architecture passes these checks in order; the first failure
is a refusal that names its check:
    sha256    the file's sha256 is the pin's, and its .sha256 sidecar records that digest for that
              file name;
    cosign    cosign verify-blob accepts its .sig and .pem for the pinned certificate identity and
              OIDC issuer.
Only then are the packages read, and refused by these:
    package   each is the fermix package of the pinned version and architecture; each carries only
              root-owned regular files and directories; the deb has a postinst and a postrm and no
              other maintainer script, and no Pre-Depends; the engine's own engine.json names the
              pinned version and commit;
    contents  the deb and the rpm hold the same files with the same modes and sha256, and the rpm's
              %post and %postun are the deb's postinst and postrm byte for byte. Nothing is exempt:
              the engine builds both families from one nFPM file, so even
              /usr/share/doc/fermix/changelog.Debian.gz is in both, identical, and is staged.
A pin engine_pin.py refuses, a stage directory that already holds files, and --dev with CI set are
refused too, as the pin, stage and dev checks.

The stage is written beside <stage dir> and renamed into place, so it is there whole or not at all.
<stage dir> may exist only if it is empty.
    tree/                      the deb's files and directories, with their modes
    maintainer/postinstall.sh  the deb's postinst, as shipped
    maintainer/postremove.sh   the deb's postrm, as shipped
    relations.json             the deb's Depends and the rpm's Requires, as declared
    engine.json                what was verified: version, tag, commit, identity, asset digests

--dev stages a local, unsigned engine package built from the engine's dev branch: no pin, no sha256
and no cosign, and engine.json says "dev": true. With an rpm the package and contents checks still
run; without one the stage has no rpm half. It refuses whenever CI is set, so no release carries it.

The pin is read through engine_pin.py, its one reader. cosign is the first on PATH: use the release
binary the engine itself pins, cosign v3.1.3 (COSIGN_RELEASES in tezra-io/fermix
scripts/release/linux_packages.py). The deb is read with the host's dpkg-deb. The rpm is read by
rpm itself inside RPM_IMAGE with no network, because neither rpm nor rpm2cpio is on an Ubuntu host;
the image is never pulled here. Every outside program runs under a time limit, past which it counts
as failed.
"""

import argparse
import contextlib
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from dataclasses import dataclass

RPM_IMAGE = "almalinux:9@sha256:9819dc675b67b595c2b59e42be7763fca1a8bb217fa5944e04daa22e9a64db16"
ENGINE_PIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "engine_pin.py")
ARCHES = {"amd64": "amd64", "x86_64": "amd64", "arm64": "arm64", "aarch64": "arm64"}
RPM_ARCHES = {"amd64": "x86_64", "arm64": "aarch64"}
PIN_TIMEOUT_SECONDS = 60
QUERY_TIMEOUT_SECONDS = 60
DPKG_TIMEOUT_SECONDS = 600
COSIGN_TIMEOUT_SECONDS = 300
DOCKER_TIMEOUT_SECONDS = 900
CHUNK_BYTES = 1 << 20
SIDECAR_FIELDS = 2
SHA256 = re.compile(r"[0-9a-f]{64}")
RELEASE_PATHS = 3
DEV_PATHS = (2, 3)
DEB_SCRIPTS = {"postinst": "postinstall.sh", "postrm": "postremove.sh"}
DEB_CONTROL_MEMBERS = ("control", "md5sums", *DEB_SCRIPTS)
RPM_SCRIPTS = (
    ("postinst", "postinstall.sh", "postin_sha256", "%post"),
    ("postrm", "postremove.sh", "postun_sha256", "%postun"),
)
RPM_SAYS = ("identity", "requires", "postin", "postun", "others")
NO_OTHER_SCRIPTLETS = "(none) (none) (none) (none) (none)\n"
# Run inside RPM_IMAGE with the rpm on stdin. Writes a tar of what rpm itself says of the package:
# its identity, its Requires, its %post and %postun, every other scriptlet and trigger it has, and
# its payload as rpm2archive unpacks it.
RPM_READER = r"""set -eu
cat > /tmp/engine.rpm
mkdir /tmp/out
cd /tmp/out
rpm -qp --qf '%{NAME}\n%{VERSION}\n%{RELEASE}\n%{ARCH}\n' /tmp/engine.rpm > identity
rpm -qp --requires /tmp/engine.rpm > requires
rpm -qp --qf '%{POSTIN}' /tmp/engine.rpm > postin
rpm -qp --qf '%{POSTUN}' /tmp/engine.rpm > postun
rpm -qp --qf '%{PREINPROG} %{PREUNPROG} %{PRETRANSPROG} %{POSTTRANSPROG} %{VERIFYSCRIPTPROG}\n' \
  /tmp/engine.rpm > others
rpm -qp --triggers /tmp/engine.rpm >> others
rpm -qp --filetriggers /tmp/engine.rpm >> others
rpm2archive < /tmp/engine.rpm > payload.tgz
tar -cf - identity requires postin postun others payload.tgz
"""
USAGE = (
    "verify_engine.py [--pin <file>] <download dir> <stage dir> <amd64|arm64>, "
    "or verify_engine.py --dev <stage dir> <local .deb> [<local .rpm>]"
)


class Refusal(Exception):
    """A refusal; its message is the sentence printed after "verify_engine: "."""


class Parser(argparse.ArgumentParser):
    """argparse, refusing a malformed command line the way every other mistake here is refused."""

    def error(self, message):
        raise Refusal(f"usage: {USAGE} ({message})")


@dataclass(frozen=True)
class Facts:
    """What the stage records as verified. A --dev build has no pin, so those fields are None."""

    dev: bool
    arch: str
    engine_version: str
    deb_asset: str
    deb_sha256: str
    rpm_asset: str | None
    rpm_sha256: str | None
    repository: str | None = None
    tag: str | None = None
    source_commit: str | None = None
    certificate_identity: str | None = None
    certificate_oidc_issuer: str | None = None


def check_failed(check, sentence):
    return Refusal(f"{check} check failed: {sentence}")


def run(command, timeout, **streams):
    """Runs one outside program to completion. Its exit status is the caller's to judge."""
    try:
        return subprocess.run(command, check=False, timeout=timeout, **streams)
    except subprocess.TimeoutExpired as error:
        raise Refusal(f"{command[0]} did not finish within {timeout} seconds") from error
    except FileNotFoundError as error:
        raise Refusal(f"{command[0]} is not installed") from error


def deb_arch(name):
    """dpkg's name for an architecture, which is what the pin is keyed by."""
    if name not in ARCHES:
        raise Refusal(
            f"no engine package is built for the architecture '{name}'; "
            "one is for amd64 and one for arm64"
        )
    return ARCHES[name]


def stream_sha256(source):
    digest = hashlib.sha256()
    for chunk in iter(lambda: source.read(CHUNK_BYTES), b""):
        digest.update(chunk)
    return digest.hexdigest()


def file_sha256(path):
    with open(path, "rb") as handle:
        return stream_sha256(handle)


def write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2)
        handle.write("\n")
    os.chmod(path, 0o644)


def pin_check(pin_args):
    """engine_pin.py says why a pin is refused on stderr; this names the check it fails."""
    if run([sys.executable, ENGINE_PIN, *pin_args, "--check"], PIN_TIMEOUT_SECONDS).returncode != 0:
        raise check_failed("pin", "the engine pin cannot be verified against")


def pin_field(pin_args, field):
    result = run(
        [sys.executable, ENGINE_PIN, *pin_args, "--get", field],
        PIN_TIMEOUT_SECONDS,
        stdout=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        raise check_failed("pin", f"the engine pin gives no {field}")
    return result.stdout.strip()


def pin_facts(pin_args, arch):
    names = (
        "repository",
        "tag",
        "engine_version",
        "source_commit",
        "certificate_identity",
        "certificate_oidc_issuer",
    )
    pin = {name: pin_field(pin_args, name) for name in names}
    packages = {
        f"{fmt}_{field}": pin_field(pin_args, f"{arch}.{fmt}.{field}")
        for fmt in ("deb", "rpm")
        for field in ("asset", "sha256")
    }
    return Facts(dev=False, arch=arch, **pin, **packages)


def sidecar_digest(sidecar, asset):
    """The digest a .sha256 sidecar records: one "<sha256>  <name>" line, for this file."""
    if not os.path.isfile(sidecar):
        raise check_failed("sha256", f"{asset} has no .sha256 beside it")
    with open(sidecar, encoding="utf-8", errors="replace") as handle:
        text = handle.read()
    if text.count("\n") != 1:
        raise check_failed("sha256", f"{asset}.sha256 is not one '<sha256>  <name>' line")
    fields = text.split()
    if len(fields) != SIDECAR_FIELDS or fields[1] != asset:
        named = fields[1] if len(fields) > 1 else "nothing"
        raise check_failed("sha256", f"{asset}.sha256 names {named}, not {asset}")
    if not SHA256.fullmatch(fields[0]):
        raise check_failed(
            "sha256", f"{asset}.sha256 records {fields[0]}, which is not a sha256 digest"
        )
    return fields[0]


def check_sha256(path, pinned):
    asset = os.path.basename(path)
    if not os.path.isfile(path):
        raise check_failed("sha256", f"{asset} is not in {os.path.dirname(path)}")
    actual = file_sha256(path)
    if actual != pinned:
        raise check_failed("sha256", f"{asset} hashes to {actual}, and the pin records {pinned}")
    recorded = sidecar_digest(f"{path}.sha256", asset)
    if recorded != pinned:
        raise check_failed(
            "sha256", f"{asset}.sha256 records {recorded}, and the pin records {pinned}"
        )


def check_signature(path, identity, issuer):
    asset = os.path.basename(path)
    for suffix in (".sig", ".pem"):
        if not os.path.isfile(f"{path}{suffix}"):
            raise check_failed("cosign", f"{asset} has no {suffix} beside it")
    result = run(
        [
            "cosign",
            "verify-blob",
            "--certificate", f"{path}.pem",
            "--signature", f"{path}.sig",
            "--certificate-identity", identity,
            "--certificate-oidc-issuer", issuer,
            path,
        ],
        COSIGN_TIMEOUT_SECONDS,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )  # fmt: skip
    if result.returncode != 0:
        print(result.stdout, end="", file=sys.stderr)
        raise check_failed("cosign", f"{asset} is not signed by {identity}, issued by {issuer}")


@contextlib.contextmanager
def staging(stage):
    """A new, empty directory beside the stage to write it in, and a work directory; both are gone
    afterwards unless the caller renamed the first into place."""
    if os.path.lexists(stage) and not (os.path.isdir(stage) and not os.listdir(stage)):
        raise check_failed(
            "stage",
            f"{stage} already holds files, and a stage is written whole into an empty place",
        )
    parent = os.path.dirname(os.path.abspath(stage))
    os.makedirs(parent, exist_ok=True)
    partial = tempfile.mkdtemp(prefix=f".{os.path.basename(stage)}.partial.", dir=parent)
    try:
        os.chmod(partial, 0o755)
        with tempfile.TemporaryDirectory() as work:
            yield partial, work
    finally:
        if os.path.exists(partial):
            shutil.rmtree(partial)


def relative_path(name, family):
    """The member's path inside the package, refusing every name that leaves or bends the tree."""
    if name.startswith("/"):
        raise check_failed("package", f"the {family} carries the absolute path {name}")
    parts = name.rstrip("/").split("/")
    if parts[0] == ".":
        parts = parts[1:]
    if any(part in ("", ".", "..") for part in parts):
        raise check_failed(
            "package", f"the {family} carries the path {name}, which leaves or bends the tree"
        )
    return "/".join(parts)


def check_entry(member, path, family):
    """Root-owned regular files and directories are all the engine ships, and all a stage holds."""
    if member.issym():
        raise check_failed(
            "package",
            f"the {family} carries the symbolic link {path}, "
            "and a stage holds files and directories only",
        )
    if not (member.isfile() or member.isdir()):
        raise check_failed(
            "package",
            f"the {family} carries {path}, which is neither a regular file nor a directory",
        )
    if (member.uid, member.gid) != (0, 0):
        raise check_failed(
            "package",
            f"the {family} carries {path} owned by {member.uid}:{member.gid}, not by root",
        )


def entry(member, digest):
    mode = f"{member.mode & 0o7777:04o}"
    if member.isdir():
        return {"kind": "directory", "mode": mode}
    return {"kind": "file", "mode": mode, "sha256": digest}


def write_file(source, target, mode):
    digest = hashlib.sha256()
    handle = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(handle, "wb") as out:
        for chunk in iter(lambda: source.read(CHUNK_BYTES), b""):
            digest.update(chunk)
            out.write(chunk)
    os.chmod(target, mode)
    return digest.hexdigest()


def take_deb_member(data, member, tree, inventory):
    path = relative_path(member.name, "deb")
    if not path:
        return
    check_entry(member, path, "deb")
    if path in inventory:
        raise check_failed("package", f"the deb carries {path} twice")
    target = os.path.join(tree, path)
    if member.isdir():
        os.mkdir(target, 0o700)
        inventory[path] = entry(member, None)
        return
    mode = member.mode & 0o7777
    inventory[path] = entry(member, write_file(data.extractfile(member), target, mode))


def write_tree(archive, tree):
    inventory = {}
    with tarfile.open(archive, mode="r|*") as data:
        for member in data:
            take_deb_member(data, member, tree, inventory)
    # Directory modes last, so that a directory without write permission still received its files.
    directories = [path for path, item in inventory.items() if item["kind"] == "directory"]
    for path in sorted(directories, reverse=True):
        os.chmod(os.path.join(tree, path), int(inventory[path]["mode"], 8))
    return inventory


def unpack_deb_tree(deb, tree, work):
    """Writes the deb's files and directories as <tree>, with every mode as shipped."""
    archive = os.path.join(work, "deb-data.tar")
    with open(archive, "wb") as out:
        result = run(["dpkg-deb", "--fsys-tarfile", deb], DPKG_TIMEOUT_SECONDS, stdout=out)
    if result.returncode != 0:
        raise check_failed("package", f"the files of {os.path.basename(deb)} cannot be read")
    os.mkdir(tree)
    os.chmod(tree, 0o755)
    try:
        return write_tree(archive, tree)
    except tarfile.TarError as error:
        raise check_failed(
            "package", f"the deb's data archive is not a readable tar: {error}"
        ) from error
    except OSError as error:
        raise check_failed(
            "package", f"the deb's data archive cannot be written as a tree: {error}"
        ) from error


def take_control_member(archive, member, members):
    path = relative_path(member.name, "deb's control archive")
    if not path:
        return
    if path not in DEB_CONTROL_MEMBERS:
        raise check_failed(
            "package", f"the deb carries the maintainer file {path}, which the stage does not carry"
        )
    if not member.isfile():
        raise check_failed("package", f"the deb's {path} is not a regular file")
    members[path] = (archive.extractfile(member).read(), member.mode & 0o7777)


def read_control_members(archive_path):
    members = {}
    with tarfile.open(archive_path, mode="r|*") as archive:
        for member in archive:
            take_control_member(archive, member, members)
    missing = [name for name in ("control", *DEB_SCRIPTS) if name not in members]
    if missing:
        raise check_failed("package", f"the deb carries no {' and no '.join(missing)}")
    return members


def fields_of(text):
    fields, name = {}, None
    for line in text.splitlines():
        if line[:1] in (" ", "\t") and name:
            fields[name] += " " + line.strip()
            continue
        name, _, value = line.partition(":")
        fields[name] = value.strip()
    return fields


def control_record(text):
    fields = fields_of(text)
    if "Pre-Depends" in fields:
        raise check_failed(
            "package",
            f"the deb declares Pre-Depends: {fields['Pre-Depends']}, "
            "which the stage does not carry",
        )
    depends = [item.strip() for item in fields.get("Depends", "").split(",") if item.strip()]
    return {
        "package": fields.get("Package"),
        "version": fields.get("Version"),
        "architecture": fields.get("Architecture"),
        "depends": depends,
    }


def stage_scripts(members, maintainer):
    os.mkdir(maintainer)
    os.chmod(maintainer, 0o755)
    for name, staged in DEB_SCRIPTS.items():
        content, mode = members[name]
        target = os.path.join(maintainer, staged)
        with open(target, "xb") as handle:
            handle.write(content)
        os.chmod(target, mode)


def read_deb_control(deb, maintainer, work):
    """Stages the deb's postinst and postrm as shipped, and returns its identity and Depends."""
    archive = os.path.join(work, "deb-control.tar")
    with open(archive, "wb") as out:
        result = run(["dpkg-deb", "--ctrl-tarfile", deb], DPKG_TIMEOUT_SECONDS, stdout=out)
    if result.returncode != 0:
        raise check_failed(
            "package", f"the maintainer scripts of {os.path.basename(deb)} cannot be read"
        )
    try:
        members = read_control_members(archive)
        record = control_record(members["control"][0].decode("utf-8"))
    except tarfile.TarError as error:
        raise check_failed(
            "package", f"the deb's control archive is not a readable tar: {error}"
        ) from error
    except UnicodeDecodeError as error:
        raise check_failed("package", f"the deb's control file is not UTF-8: {error}") from error
    stage_scripts(members, maintainer)
    return record


def deb_field(deb, field):
    result = run(
        ["dpkg-deb", "--field", deb, field],
        QUERY_TIMEOUT_SECONDS,
        stdout=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        raise check_failed("package", f"{deb} is not a deb dpkg-deb can read")
    return result.stdout.strip()


def take_rpm_member(payload, member, inventory):
    path = relative_path(member.name, "rpm")
    check_entry(member, path, "rpm")
    if path in inventory:
        raise check_failed("package", f"the rpm carries {path} twice")
    digest = None if member.isdir() else stream_sha256(payload.extractfile(member))
    inventory[path] = entry(member, digest)


def payload_inventory(stream):
    inventory = {}
    with tarfile.open(fileobj=stream, mode="r|*") as payload:
        for member in payload:
            take_rpm_member(payload, member, inventory)
    return inventory


def read_reader_output(path):
    with tarfile.open(path, "r:") as outer:
        said = {name: outer.extractfile(name).read() for name in RPM_SAYS}
        files = payload_inventory(outer.extractfile("payload.tgz"))
    return said, files


def rpm_record(said, files):
    identity = said["identity"].decode("utf-8").split("\n")
    others = said["others"].decode("utf-8")
    if others != NO_OTHER_SCRIPTLETS:
        raise check_failed(
            "package",
            f"the rpm carries scriptlets or triggers beyond %post and %postun: {others.strip()}",
        )
    requires = said["requires"].decode("utf-8").splitlines()
    return {
        "name": identity[0],
        "version": identity[1],
        "release": identity[2],
        "arch": identity[3],
        "requires": [line.strip() for line in requires if line.strip()],
        "postin_sha256": hashlib.sha256(said["postin"]).hexdigest(),
        "postun_sha256": hashlib.sha256(said["postun"]).hexdigest(),
        "files": files,
    }


def read_rpm(rpm, work):
    """What rpm itself, inside RPM_IMAGE, says the package declares and holds."""
    inspect = run(
        ["docker", "image", "inspect", RPM_IMAGE],
        QUERY_TIMEOUT_SECONDS,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if inspect.returncode != 0:
        raise Refusal(
            "the rpm reader image is not here, and verify_engine.py never pulls; "
            f"run: docker pull {RPM_IMAGE}"
        )
    output = os.path.join(work, "rpm.tar")
    with open(rpm, "rb") as source, open(output, "wb") as out:
        result = run(
            ["docker", "run", "--rm", "-i", "--network", "none", "--pull", "never",
             RPM_IMAGE, "sh", "-c", RPM_READER],
            DOCKER_TIMEOUT_SECONDS,
            stdin=source,
            stdout=out,
        )  # fmt: skip
    if result.returncode != 0:
        raise check_failed("package", f"{os.path.basename(rpm)} is not an rpm that rpm can read")
    try:
        said, files = read_reader_output(output)
    except (tarfile.TarError, KeyError) as error:
        raise check_failed(
            "package", f"rpm's own reading of the package is incomplete: {error}"
        ) from error
    return rpm_record(said, files)


def check_identities(control, rpm, facts):
    wanted = {"package": "fermix", "version": facts.engine_version, "architecture": facts.arch}
    for key, value in wanted.items():
        if control[key] != value:
            raise check_failed(
                "package", f"the deb's control says {key} {control[key]}, and {value} was expected"
            )
    if rpm is None:
        return
    wanted = {"name": "fermix", "version": facts.engine_version, "arch": RPM_ARCHES[facts.arch]}
    for key, value in wanted.items():
        if rpm[key] != value:
            raise check_failed(
                "package", f"the rpm says {key} {rpm[key]}, and {value} was expected"
            )


def check_engine(tree, facts):
    """The engine's own record of its build, which the pin's version and commit must match."""
    try:
        with open(os.path.join(tree, "usr/share/fermix/engine.json"), encoding="utf-8") as handle:
            engine = json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        raise check_failed(
            "package", f"the deb carries no readable usr/share/fermix/engine.json: {error}"
        ) from error
    if not isinstance(engine, dict) or engine.get("product_version") != facts.engine_version:
        raise check_failed(
            "package", f"the engine's engine.json is not of version {facts.engine_version}"
        )
    commit = engine.get("source_commit")
    if facts.source_commit is not None and commit != facts.source_commit:
        raise check_failed(
            "package",
            f"the engine was built from {commit}, and the pin names {facts.source_commit}",
        )
    return commit, engine.get("build_id")


def describe(item):
    if item["kind"] == "directory":
        return f"a directory of mode {item['mode']}"
    return f"a file of mode {item['mode']} and sha256 {item['sha256']}"


def difference(path, deb, rpm):
    if deb == rpm:
        return []
    if rpm is None and deb["kind"] == "directory":
        # nFPM's rpm carries an entry only for a directory the package declares, and the deb carries
        # every parent, so a directory the rpm lacks is not a difference in what is installed.
        return []
    if deb is None:
        return [f"{path} is in the rpm and not in the deb"]
    if rpm is None:
        return [f"{path} is in the deb and not in the rpm"]
    return [f"{path} is {describe(deb)} in the deb and {describe(rpm)} in the rpm"]


def compare(deb_files, rpm, maintainer):
    differences = []
    for path in sorted(set(deb_files) | set(rpm["files"])):
        differences += difference(path, deb_files.get(path), rpm["files"].get(path))
    if differences:
        raise check_failed("contents", "; ".join(differences))
    for deb_name, staged, key, rpm_name in RPM_SCRIPTS:
        if file_sha256(os.path.join(maintainer, staged)) != rpm[key]:
            raise check_failed(
                "contents", f"the deb's {deb_name} is not the rpm's {rpm_name} scriptlet"
            )


def records(facts, control, rpm, commit, build_id):
    engine = {
        "schema_version": 1,
        "dev": facts.dev,
        "repository": facts.repository,
        "tag": facts.tag,
        "engine_version": facts.engine_version,
        "source_commit": commit,
        "build_id": build_id,
        "certificate_identity": facts.certificate_identity,
        "certificate_oidc_issuer": facts.certificate_oidc_issuer,
        "arch": facts.arch,
        "deb": {"asset": facts.deb_asset, "sha256": facts.deb_sha256},
        "rpm": None if rpm is None else {"asset": facts.rpm_asset, "sha256": facts.rpm_sha256},
    }
    relations = {
        "deb": {"asset": facts.deb_asset, "depends": control["depends"]},
        "rpm": None if rpm is None else {"asset": facts.rpm_asset, "requires": rpm["requires"]},
    }
    return engine, relations


def stage_packages(partial, work, deb, rpm, facts):
    """Reads both packages, holds them against each other and against the facts, and writes the
    stage's contents into <partial>. The rpm is None for a --dev build that has none."""
    deb_files = unpack_deb_tree(deb, os.path.join(partial, "tree"), work)
    control = read_deb_control(deb, os.path.join(partial, "maintainer"), work)
    rpm_says = None if rpm is None else read_rpm(rpm, work)
    check_identities(control, rpm_says, facts)
    commit, build_id = check_engine(os.path.join(partial, "tree"), facts)
    if rpm_says is not None:
        compare(deb_files, rpm_says, os.path.join(partial, "maintainer"))
    engine, relations = records(facts, control, rpm_says, commit, build_id)
    write_json(os.path.join(partial, "relations.json"), relations)
    write_json(os.path.join(partial, "engine.json"), engine)


def verify_release(pin_args, download, stage, arch_name):
    pin_check(pin_args)
    arch = deb_arch(arch_name)
    if not os.path.isdir(download):
        raise Refusal(f"there is no download directory at {download}")
    if shutil.which("cosign") is None:
        raise Refusal("cosign is not on PATH, and every package's signature is checked")
    facts = pin_facts(pin_args, arch)
    deb = os.path.join(download, facts.deb_asset)
    rpm = os.path.join(download, facts.rpm_asset)
    identity, issuer = facts.certificate_identity, facts.certificate_oidc_issuer
    with staging(stage) as (partial, work):
        check_sha256(deb, facts.deb_sha256)
        check_signature(deb, identity, issuer)
        check_sha256(rpm, facts.rpm_sha256)
        check_signature(rpm, identity, issuer)
        stage_packages(partial, work, deb, rpm, facts)
        os.rename(partial, stage)
    print(
        f"verify_engine: {facts.deb_asset} and {facts.rpm_asset} of {facts.tag} "
        f"verified and staged in {stage}"
    )


def verify_dev(stage, deb, rpm):
    if "CI" in os.environ:
        raise check_failed(
            "dev", "CI is set, and --dev stages an unsigned engine that no release may carry"
        )
    if not os.path.isfile(deb):
        raise check_failed("package", f"there is no deb at {deb}")
    if rpm is not None and not os.path.isfile(rpm):
        raise check_failed("package", f"there is no rpm at {rpm}")
    facts = Facts(
        dev=True,
        arch=deb_arch(deb_field(deb, "Architecture")),
        engine_version=deb_field(deb, "Version"),
        deb_asset=os.path.basename(deb),
        deb_sha256=file_sha256(deb),
        rpm_asset=None if rpm is None else os.path.basename(rpm),
        rpm_sha256=None if rpm is None else file_sha256(rpm),
    )
    with staging(stage) as (partial, work):
        unsigned = deb if rpm is None else f"{deb} and {rpm}"
        print(
            f"verify_engine: --dev: {unsigned} are unsigned and unpinned; "
            "this stage is a development build",
            file=sys.stderr,
        )
        stage_packages(partial, work, deb, rpm, facts)
        os.rename(partial, stage)
    print(
        f"verify_engine: development build {facts.engine_version} ({facts.arch}) staged in {stage}"
    )


def parse_arguments(argv):
    parser = Parser(prog="verify_engine.py", add_help=False)
    parser.add_argument("--pin")
    parser.add_argument("--dev", action="store_true")
    parser.add_argument("paths", nargs="*")
    arguments = parser.parse_intermixed_args(argv)
    count = len(arguments.paths)
    if arguments.dev and (arguments.pin is not None or count not in DEV_PATHS):
        raise Refusal(f"usage: {USAGE}")
    if not arguments.dev and count != RELEASE_PATHS:
        raise Refusal(f"usage: {USAGE}")
    return arguments


def main(argv):
    try:
        arguments = parse_arguments(argv)
        paths = [os.path.normpath(path) for path in arguments.paths]
        if arguments.dev:
            verify_dev(paths[0], paths[1], paths[2] if len(paths) == DEV_PATHS[1] else None)
        else:
            pin_args = [] if arguments.pin is None else ["--pin", arguments.pin]
            verify_release(pin_args, paths[0], paths[1], arguments.paths[2])
    except Refusal as refusal:
        print(f"verify_engine: {refusal}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
