#!/usr/bin/env python3
"""fermix-desktop's update feed, format 1: one release's entry, the cumulative feed, its signature.

    update_feed.py item --packages <dir> --version <X.Y.Z> --pin <PIN.json> --notes <notes.md>
                        --base-url <https url> --published-at <time> --out <appcast-item.json>
    update_feed.py merge --item <appcast-item.json> [--previous <appcast.json>]
                         [--floor <appcast.json>] --keys <keys.json> --generated-at <time>
                         --out <payload.json>
    update_feed.py sign --payload <payload.json> --keys <keys.json> --out <appcast.json> < <key.pem>
    update_feed.py verify --feed <appcast.json> --keys <keys.json> [--decoded <file>]

<time> is UTC, YYYY-MM-DDTHH:MM:SSZ. <keys.json> is desktop/packaging/update-keys.json, the public
keys the app compiles in, each named by its key_id: the current key, which signs, and a standby one
for a rotation.

The app fetches https://fermix.ai/linux/appcast.json, the envelope: {"format": 1, "key_id": <id>,
"payload": <base64 of the payload's bytes>, "signature": <base64 of their ed25519 signature>}. It
refuses an unknown key_id, any other field, and a signature that does not verify over the payload's
bytes exactly as they decode; nothing is canonicalised. The payload is JSON: {"format": 1,
"product": "fermix-desktop", "generated_at": <time>, "releases": [...]}, with 1 to 200 releases,
newest first. A release names its version, published_at, critical, minimum_installed_version, the
engine it carries, 1 to 8 notes, and for amd64 and arm64 the deb and the rpm: each one's file name,
https url, size and sha256.

item writes one release from the files themselves: the four packages in <dir>, named as
build_packages.sh names them, measured and hashed here; the engine from the pin, which
engine_pin.py --check must accept; and the notes from desktop/packaging/notes/<version>.md, one
"- " bullet per line. That file's optional front matter, between two "---" lines, is the only
source of `critical: true|false` and `minimum_installed_version: X.Y.Z`. The url is the base url,
then "/", then the file's name.

merge puts the item in front of the releases of the previous feed, which must verify against
<keys.json>. It refuses a version that is not higher than the newest one there, and a generated_at
that is not later than the previous feed's, since the app refuses an older feed. Without
--previous, the item is the first release of a new feed. It keeps the newest 200. --floor is the
feed the last published release carries, which must verify too: the previous feed must carry
every version the floor carries, and must not be older than it. So a feed that a release once
published never starts again, and a served feed that went back to an older one is refused.

sign signs the payload's bytes as they are, as the keys file's current key, with OpenSSL 3. The
private key PEM comes on standard input, which openssl pkeyutl inherits and reads as /dev/stdin, so
it is never an argument, a file or an environment variable this script reads, and never printed.
The signature must verify against the current key's public key, or nothing is written: a secret
that is not that key's is refused.

verify checks an envelope as the app does, and its payload against every rule above. --decoded
writes {"key_id": <id>, "payload": <the decoded payload>} as JSON.

Every document this reads is held to the rules the writer follows, so a release this refuses is
never published as a guess.
"""

import argparse
import base64
import binascii
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import unicodedata
from urllib.parse import urlsplit

FORMAT = 1
PRODUCT = "fermix-desktop"
# The app stops reading at 1 MiB.
MAX_FEED_BYTES = 1024 * 1024
MAX_RELEASES = 200
MAX_NOTES = 8
MAX_NOTE_CHARACTERS = 300
MAX_PACKAGE_BYTES = 512 * 1024 * 1024
MAX_URL_CHARACTERS = 2048
SIGNATURE_BYTES = 64
PUBLIC_KEY_BYTES = 32
# X.Y.Z, plain numbers with no leading zeros.
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
TIME_FORMAT = "%Y-%m-%dT%H:%M:%SZ"
TIME = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z")
KEY_ID = re.compile(r"[a-z0-9-]{1,64}")
SHA256 = re.compile(r"[0-9a-f]{64}")
COMMIT = re.compile(r"[0-9a-f]{40}")
# The characters a url carries unescaped, less "?" and "#": a package url has no query or fragment.
URL_CHARACTERS = re.compile(r"[A-Za-z0-9._~:/!$&'()*+,;=%@-]+")
# dpkg's name for each architecture, and rpm's.
ARCHES = {"amd64": "x86_64", "arm64": "aarch64"}
FORMATS = ("deb", "rpm")
ENVELOPE_KEYS = ("format", "key_id", "payload", "signature")
ROLES = ("current", "standby")
FRONT_MATTER = "---"
BULLET = "- "
# Markup the app does not draw: a note is a plain sentence, with `code` its only markup.
MARKUP = "*_[]<>\\"
SENTENCE_ENDS = ".!?"
# Characters that would hide or reorder what a note says: line and paragraph separators, and the
# bidirectional controls. Control characters and surrogates are refused by category.
HIDDEN = {chr(c) for c in (0x061C, 0x200E, 0x200F, 0x2028, 0x2029)}
HIDDEN |= {chr(c) for c in range(0x202A, 0x202F)} | {chr(c) for c in range(0x2066, 0x206A)}
# The DER that wraps an ed25519 public key (SubjectPublicKeyInfo) for OpenSSL.
ED25519_SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")
OPENSSL_TIMEOUT_SECONDS = 60
PIN_TIMEOUT_SECONDS = 60
ENGINE_PIN = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "scripts", "engine_pin.py"
)
USAGE = "update_feed.py item | merge | sign | verify, each with its flags (see the header)"


class Refusal(Exception):
    """A refusal; its message is the sentence printed after "update_feed: "."""


class Parser(argparse.ArgumentParser):
    """argparse, refusing a malformed command line the way every other mistake here is refused."""

    def error(self, message):
        raise Refusal(f"usage: {USAGE} ({message})")


# Reading and writing


def read_bytes(path, what, limit):
    try:
        with open(path, "rb") as handle:
            data = handle.read(limit + 1)
    except OSError as error:
        raise Refusal(f"cannot read {what} at {path}: {error.strerror}") from error
    if len(data) > limit:
        raise Refusal(f"{what} at {path} is larger than {limit} bytes")
    return data


def shown(value):
    """A value as the JSON a refusal quotes it in."""
    return json.dumps(value, ensure_ascii=False)


def listed(words):
    """The words as a sentence lists them: a; a and b; a, b and c."""
    if len(words) == 1:
        return words[0]
    return f"{', '.join(words[:-1])} and {words[-1]}"


def no_duplicates(pairs):
    keys = [key for key, _ in pairs]
    repeated = sorted({key for key in keys if keys.count(key) > 1})
    if repeated:
        raise ValueError(f"the key {shown(repeated[0])} appears twice in one object")
    return dict(pairs)


def no_constant(name):
    raise ValueError(f"{name} is not a JSON number")


def decode_json(data, what):
    """Strict UTF-8 JSON: no byte-order mark, no repeated key, no NaN or Infinity."""
    try:
        text = data.decode("utf-8")
        return json.loads(text, object_pairs_hook=no_duplicates, parse_constant=no_constant)
    except UnicodeDecodeError as error:
        raise Refusal(f"{what} is not UTF-8: {error}") from error
    except ValueError as error:
        raise Refusal(f"{what} is not valid JSON: {error}") from error


def read_json(path, what, limit=MAX_FEED_BYTES):
    return decode_json(read_bytes(path, what, limit), f"{what} at {path}")


def write_file(path, data):
    """Replaces <path> in one rename, so a reader never sees half a file."""
    directory = os.path.dirname(os.path.abspath(path))
    if not os.path.isdir(directory):
        raise Refusal(f"there is no directory to write {path} into")
    handle, staged = tempfile.mkstemp(prefix=".update_feed.", dir=directory)
    try:
        with os.fdopen(handle, "wb") as out:
            out.write(data)
        os.chmod(staged, 0o644)
        os.replace(staged, path)
    finally:
        if os.path.exists(staged):
            os.remove(staged)


def pretty(document):
    return (json.dumps(document, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def compact(document):
    return json.dumps(document, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


# The rules every document is held to


def is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


def version_key(version):
    return tuple(int(part) for part in version.split("."))


def check_version(value, what):
    if not isinstance(value, str) or not VERSION.fullmatch(value):
        raise Refusal(f"{what} is {shown(value)}, which is not a release version X.Y.Z")
    return value


def check_time(value, what):
    if not isinstance(value, str) or not TIME.fullmatch(value):
        raise Refusal(f"{what} is {shown(value)}, which is not a UTC time YYYY-MM-DDTHH:MM:SSZ")
    try:
        datetime.datetime.strptime(value, TIME_FORMAT)
    except ValueError as error:
        raise Refusal(f"{what} is {value}, which is not a real time") from error
    return value


def check_object(value, what):
    if not isinstance(value, dict):
        raise Refusal(f"{what} is not a JSON object")
    return value


def strict_base64(value, what):
    """Standard base64 with padding, in its one canonical spelling."""
    if not isinstance(value, str):
        raise Refusal(f"{what} is not a string of base64")
    try:
        decoded = base64.b64decode(value.encode("ascii"), validate=True)
    except (UnicodeEncodeError, binascii.Error) as error:
        raise Refusal(f"{what} is not standard base64 with padding: {error}") from error
    if base64.b64encode(decoded).decode("ascii") != value:
        raise Refusal(f"{what} is not base64 in its canonical form")
    return decoded


def code_spans_removed(note, what):
    parts = note.split("`")
    if len(parts) % 2 == 0:
        raise Refusal(f"{what} opens a `code` span it does not close")
    return "".join(parts[0::2])


def check_note(note, what):
    if not isinstance(note, str) or not 1 <= len(note) <= MAX_NOTE_CHARACTERS:
        raise Refusal(f"{what} is not a sentence of 1 to {MAX_NOTE_CHARACTERS} characters")
    if note != note.strip():
        raise Refusal(f"{what} begins or ends with a space")
    hidden = [c for c in note if c in HIDDEN or unicodedata.category(c) in ("Cc", "Cs")]
    if hidden:
        raise Refusal(f"{what} carries the hidden or control character U+{ord(hidden[0]):04X}")
    markup = [c for c in code_spans_removed(note, what) if c in MARKUP]
    if markup:
        raise Refusal(
            f"{what} carries {shown(markup[0])} outside a `code` span: notes are plain sentences, "
            "and the app draws no markup but `code`"
        )
    if note[-1] not in SENTENCE_ENDS:
        raise Refusal(f"{what} does not end a sentence with '.', '!' or '?'")
    return note


def check_notes(notes, what):
    if not isinstance(notes, list) or not 1 <= len(notes) <= MAX_NOTES:
        raise Refusal(f"{what} has no list of 1 to {MAX_NOTES} notes")
    for number, note in enumerate(notes, 1):
        check_note(note, f"note {number} of {what}")
    return notes


def check_url(url, file_name, what):
    if not isinstance(url, str) or not 1 <= len(url) <= MAX_URL_CHARACTERS:
        raise Refusal(f"{what} has no url of at most {MAX_URL_CHARACTERS} characters")
    if not URL_CHARACTERS.fullmatch(url):
        raise Refusal(f"{what} has the url {url}, which carries a character a url escapes")
    parts = urlsplit(url)
    if parts.scheme != "https" or not parts.hostname or "@" in parts.netloc or ":" in parts.netloc:
        raise Refusal(
            f"{what} has the url {url}, which is not https to a host, with no user or port"
        )
    if not parts.path.endswith(f"/{file_name}"):
        raise Refusal(f"{what} has the url {url}, which does not end in /{file_name}")
    return url


def file_name(version, arch, fmt):
    """The name build_packages.sh gives the package."""
    if fmt == "deb":
        return f"fermix-desktop_{version}_{arch}.deb"
    return f"fermix-desktop-{version}-1.{ARCHES[arch]}.rpm"


def check_package(entry, version, arch, fmt, what):
    check_object(entry, what)
    wanted = file_name(version, arch, fmt)
    if entry.get("file") != wanted:
        raise Refusal(
            f"{what} names the file {shown(entry.get('file'))}, and {version}'s is {wanted}"
        )
    check_url(entry.get("url"), wanted, what)
    size = entry.get("size")
    if not is_int(size) or not 1 <= size <= MAX_PACKAGE_BYTES:
        raise Refusal(f"{what} has the size {shown(size)}, not 1 to {MAX_PACKAGE_BYTES} bytes")
    if not isinstance(entry.get("sha256"), str) or not SHA256.fullmatch(entry["sha256"]):
        raise Refusal(f"{what} has the sha256 {shown(entry.get('sha256'))}, not 64 lowercase hex")


def check_engine(engine, what):
    check_object(engine, what)
    version = check_version(engine.get("version"), f"the version of {what}")
    if engine.get("tag") != f"v{version}":
        raise Refusal(
            f"{what} names the tag {shown(engine.get('tag'))}, and {version}'s is v{version}"
        )
    commit = engine.get("source_commit")
    if not isinstance(commit, str) or not COMMIT.fullmatch(commit):
        raise Refusal(f"{what} has the source_commit {shown(commit)}, not 40 lowercase hex")


def check_release(release, what):
    check_object(release, what)
    version = check_version(release.get("version"), f"the version of {what}")
    what = f"{what}, {version}"
    check_time(release.get("published_at"), f"published_at of {what}")
    if not isinstance(release.get("critical"), bool):
        raise Refusal(
            f"{what} says critical is {shown(release.get('critical'))}, not true or false"
        )
    if "minimum_installed_version" not in release:
        raise Refusal(f"{what} has no minimum_installed_version, which is null or X.Y.Z")
    minimum = release["minimum_installed_version"]
    if minimum is not None:
        check_version(minimum, f"minimum_installed_version of {what}")
    if minimum is not None and version_key(minimum) > version_key(version):
        raise Refusal(f"{what} asks for {minimum} installed first, which is newer than itself")
    check_engine(release.get("engine"), f"the engine of {what}")
    check_notes(release.get("notes"), what)
    packages = check_object(release.get("packages"), f"the packages of {what}")
    for arch in ARCHES:
        entries = check_object(packages.get(arch), f"the {arch} packages of {what}")
        for fmt in FORMATS:
            check_package(entries.get(fmt), version, arch, fmt, f"the {arch} {fmt} of {what}")
    return release


def check_payload(payload, what):
    check_object(payload, what)
    if not is_int(payload.get("format")) or payload["format"] != FORMAT:
        raise Refusal(
            f"{what} is format {shown(payload.get('format'))}, and this is format {FORMAT}"
        )
    if payload.get("product") != PRODUCT:
        raise Refusal(f"{what} is for {shown(payload.get('product'))}, not {PRODUCT}")
    check_time(payload.get("generated_at"), f"generated_at of {what}")
    releases = payload.get("releases")
    if not isinstance(releases, list) or not 1 <= len(releases) <= MAX_RELEASES:
        raise Refusal(f"{what} does not list 1 to {MAX_RELEASES} releases")
    for number, release in enumerate(releases, 1):
        check_release(release, f"release {number} of {what}")
    versions = [version_key(release["version"]) for release in releases]
    for newer, older in zip(versions, versions[1:]):
        if newer <= older:
            raise Refusal(f"the releases of {what} are not newest first with no version twice")
    return payload


# The keys


def read_keys(path):
    """{key_id: (role, public key bytes)} from the keys file, held to its rules."""
    document = check_object(read_json(path, "the keys file"), f"the keys file at {path}")
    if (
        set(document) != {"format", "keys"}
        or not is_int(document["format"])
        or document["format"] != FORMAT
    ):
        raise Refusal(f"the keys file at {path} is not {{format: {FORMAT}, keys: [...]}}")
    entries = document["keys"]
    if not isinstance(entries, list) or not 1 <= len(entries) <= len(ROLES):
        raise Refusal(f"the keys file at {path} does not list 1 or 2 keys")
    keys = {}
    for entry in entries:
        key_id, role, public = key_entry(entry, path)
        if key_id in keys or public in [known for _, known in keys.values()]:
            raise Refusal(f"the keys file at {path} lists {key_id} or its public key twice")
        keys[key_id] = (role, public)
    roles = sorted(role for role, _ in keys.values())
    if roles.count("current") != 1 or roles.count("standby") > 1:
        raise Refusal(
            f"the keys file at {path} names {shown(roles)}, not one current and one standby"
        )
    return keys


def key_entry(entry, path):
    what = f"a key in the keys file at {path}"
    check_object(entry, what)
    if set(entry) != {"key_id", "role", "public_key"}:
        raise Refusal(f"{what} is not {{key_id, role, public_key}}")
    if not isinstance(entry["key_id"], str) or not KEY_ID.fullmatch(entry["key_id"]):
        raise Refusal(f"{what} has the key_id {shown(entry['key_id'])}, not [a-z0-9-]{{1,64}}")
    if entry["role"] not in ROLES:
        raise Refusal(f"{what} has the role {shown(entry['role'])}, not current or standby")
    public = strict_base64(entry["public_key"], f"the public key of {entry['key_id']}")
    if len(public) != PUBLIC_KEY_BYTES:
        raise Refusal(f"the public key of {entry['key_id']} is not {PUBLIC_KEY_BYTES} bytes")
    return entry["key_id"], entry["role"], public


def current_key(keys):
    return next((key_id, public) for key_id, (role, public) in keys.items() if role == "current")


# OpenSSL, which signs and checks


def openssl(arguments, what, stdin=subprocess.DEVNULL):
    try:
        return subprocess.run(
            ["openssl", *arguments],
            stdin=stdin,
            capture_output=True,
            timeout=OPENSSL_TIMEOUT_SECONDS,
            check=False,
        )
    except FileNotFoundError as error:
        raise Refusal(f"OpenSSL 3 is needed to {what}, and there is no openssl") from error
    except subprocess.TimeoutExpired as error:
        raise Refusal(f"openssl did not {what} in {OPENSSL_TIMEOUT_SECONDS} seconds") from error


def put(path, data):
    """Writes a scratch file for openssl to read."""
    with open(path, "wb") as handle:
        handle.write(data)


def one_line(data):
    return " ".join(data.decode("utf-8", "replace").split()) or "it printed nothing"


def signature_verifies(public, payload, signature):
    der = ED25519_SPKI_PREFIX + public
    pem = b"-----BEGIN PUBLIC KEY-----\n" + base64.b64encode(der) + b"\n-----END PUBLIC KEY-----\n"
    with tempfile.TemporaryDirectory(prefix="update_feed.") as work:
        paths = {name: os.path.join(work, name) for name in ("key.pem", "payload", "signature")}
        for name, data in (("key.pem", pem), ("payload", payload), ("signature", signature)):
            put(paths[name], data)
        result = openssl(
            ["pkeyutl", "-verify", "-pubin", "-inkey", paths["key.pem"], "-rawin",
             "-in", paths["payload"], "-sigfile", paths["signature"]],
            "check a signature",
        )  # fmt: skip
    if result.returncode == 0 and result.stdout == b"Signature Verified Successfully\n":
        return True
    if result.returncode == 1 and result.stdout == b"Signature Verification Failure\n":
        return False
    raise Refusal(
        f"openssl could not check the signature: {one_line(result.stdout + result.stderr)}"
    )


def sign_bytes(payload):
    """The ed25519 signature of <payload> by the key openssl reads from this process's stdin."""
    if sys.stdin is None or sys.stdin.isatty():
        raise Refusal("the private key comes as PEM on standard input, and none is there")
    with tempfile.TemporaryDirectory(prefix="update_feed.") as work:
        path = os.path.join(work, "payload")
        put(path, payload)
        result = openssl(
            ["pkeyutl", "-sign", "-rawin", "-inkey", "/dev/stdin", "-in", path],
            "sign the payload",
            stdin=sys.stdin.fileno(),
        )
    if result.returncode != 0:
        raise Refusal(f"openssl could not sign the payload: {one_line(result.stderr)}")
    if len(result.stdout) != SIGNATURE_BYTES:
        raise Refusal(
            f"openssl wrote a {len(result.stdout)}-byte signature, and ed25519's is "
            f"{SIGNATURE_BYTES}: the key on standard input is not an ed25519 key"
        )
    return result.stdout


# The envelope


def open_envelope(data, keys, what):
    """(key_id, payload bytes, payload) of a feed the keys sign, held to every rule."""
    envelope = decode_json(data, what)
    check_object(envelope, what)
    if sorted(envelope) != sorted(ENVELOPE_KEYS):
        raise Refusal(
            f"{what} has the fields {shown(sorted(envelope))}, not {shown(list(ENVELOPE_KEYS))}"
        )
    if not is_int(envelope["format"]) or envelope["format"] != FORMAT:
        raise Refusal(f"{what} is format {shown(envelope['format'])}, and this is format {FORMAT}")
    key_id = envelope["key_id"]
    if not isinstance(key_id, str) or not KEY_ID.fullmatch(key_id):
        raise Refusal(f"{what} has the key_id {shown(key_id)}, not [a-z0-9-]{{1,64}}")
    if key_id not in keys:
        raise Refusal(f"{what} is signed by {key_id}, which the keys file does not list")
    payload = strict_base64(envelope["payload"], f"the payload of {what}")
    signature = strict_base64(envelope["signature"], f"the signature of {what}")
    if len(signature) != SIGNATURE_BYTES:
        raise Refusal(f"the signature of {what} is not {SIGNATURE_BYTES} bytes")
    if not signature_verifies(keys[key_id][1], payload, signature):
        raise Refusal(f"the signature of {what} does not verify against {key_id}")
    decoded = check_payload(
        decode_json(payload, f"the payload of {what}"), f"the payload of {what}"
    )
    return key_id, payload, decoded


def envelope_bytes(key_id, payload, signature):
    envelope = {
        "format": FORMAT,
        "key_id": key_id,
        "payload": base64.b64encode(payload).decode("ascii"),
        "signature": base64.b64encode(signature).decode("ascii"),
    }
    data = pretty(envelope)
    if len(data) > MAX_FEED_BYTES:
        raise Refusal(f"the signed feed is {len(data)} bytes, over the app's {MAX_FEED_BYTES}")
    return data


# item


def read_notes(path, version):
    """(critical, minimum_installed_version, notes) from the release's notes file."""
    data = read_bytes(path, f"the notes of {version}", MAX_FEED_BYTES)
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise Refusal(f"the notes at {path} are not UTF-8: {error}") from error
    if not text.endswith("\n") or "\r" in text:
        raise Refusal(f"the notes at {path} do not end every line with one newline")
    lines = text[:-1].split("\n")
    settings = {}
    if lines[0] == FRONT_MATTER:
        if FRONT_MATTER not in lines[1:]:
            raise Refusal(f"the front matter of {path} opens with --- and never closes")
        end = lines.index(FRONT_MATTER, 1)
        settings = front_matter(lines[1:end], path)
        lines = lines[end + 1 :]
    notes = []
    for number, line in enumerate(lines, 1):
        if not line.startswith(BULLET):
            raise Refusal(
                f"line {number} of the notes in {path} is not a '- ' bullet: {shown(line)}"
            )
        notes.append(line[len(BULLET) :])
    check_notes(notes, f"the notes at {path}")
    return settings.get("critical", False), settings.get("minimum_installed_version"), notes


def front_matter(lines, path):
    settings = {}
    for line in lines:
        key, separator, value = line.partition(": ")
        if not separator or key in settings:
            raise Refusal(
                f"the front matter of {path} has the line {shown(line)}, not one new key: value"
            )
        if key == "critical" and value in ("true", "false"):
            settings[key] = value == "true"
        elif key == "minimum_installed_version":
            settings[key] = check_version(value, f"minimum_installed_version in {path}")
        else:
            raise Refusal(
                f"the front matter of {path} has the line {shown(line)}: it takes "
                "critical: true|false and minimum_installed_version: X.Y.Z, and nothing else"
            )
    return settings


def read_engine(pin):
    """The engine block, from a pin engine_pin.py --check accepts whole."""
    try:
        result = subprocess.run(
            [sys.executable, ENGINE_PIN, "--pin", pin, "--check"],
            stdin=subprocess.DEVNULL,
            timeout=PIN_TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise Refusal(
            f"engine_pin.py did not check {pin} in {PIN_TIMEOUT_SECONDS} seconds"
        ) from error
    if result.returncode != 0:
        raise Refusal(f"the engine pin at {pin} is not one a release can carry")
    document = read_json(pin, "the engine pin")
    return {
        "version": document["engine_version"],
        "tag": document["tag"],
        "source_commit": document["source_commit"],
    }


def measured(packages, name, base_url):
    path = os.path.join(packages, name)
    if not os.path.isfile(path):
        raise Refusal(f"there is no package {name} in {packages}")
    digest = hashlib.sha256()
    size = 0
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
            size += len(block)
    return {"file": name, "url": f"{base_url.rstrip('/')}/{name}", "size": size,
            "sha256": digest.hexdigest()}  # fmt: skip


def run_item(arguments):
    version = check_version(arguments.version, "the version")
    published_at = check_time(arguments.published_at, "--published-at")
    critical, minimum, notes = read_notes(arguments.notes, version)
    release = {
        "version": version,
        "published_at": published_at,
        "critical": critical,
        "minimum_installed_version": minimum,
        "engine": read_engine(arguments.pin),
        "notes": notes,
        "packages": {
            arch: {
                fmt: measured(arguments.packages, file_name(version, arch, fmt), arguments.base_url)
                for fmt in FORMATS
            }
            for arch in ARCHES
        },
    }
    check_release(release, "the item")
    write_file(arguments.out, pretty(release))
    print(f"update_feed: {arguments.out}, {version} with engine {release['engine']['version']}")


# merge, sign and verify


def read_feed(path, keys, what):
    """The payload of the feed at <path>, which <keys> must sign."""
    data = read_bytes(path, what, MAX_FEED_BYTES)
    return open_envelope(data, keys, f"{what} at {path}")[2]


def check_floor(previous, floor, what, floor_what):
    """Refuses a previous feed that is missing, older than the floor, or without its releases."""
    versions = [release["version"] for release in floor["releases"]]
    if previous is None:
        raise Refusal(f"there is no previous feed, but {floor_what} carries {listed(versions)}")
    if previous["generated_at"] < floor["generated_at"]:
        raise Refusal(
            f"{what} was generated {previous['generated_at']}, "
            f"before {floor_what} ({floor['generated_at']})"
        )
    carried = {release["version"] for release in previous["releases"]}
    missing = [version for version in versions if version not in carried]
    if missing:
        raise Refusal(f"{what} lacks {listed(missing)}, which {floor_what} carries")


def run_merge(arguments):
    item = check_release(read_json(arguments.item, "the item"), f"the item at {arguments.item}")
    generated_at = check_time(arguments.generated_at, "--generated-at")
    keys = read_keys(arguments.keys)
    what = f"the previous feed at {arguments.previous}"
    previous = None
    if arguments.previous is not None:
        previous = read_feed(arguments.previous, keys, "the previous feed")
        if generated_at <= previous["generated_at"]:
            raise Refusal(f"{generated_at} is not later than {what}'s {previous['generated_at']}")
    if arguments.floor is not None:
        floor = read_feed(arguments.floor, keys, "the floor feed")
        check_floor(previous, floor, what, f"the floor feed at {arguments.floor}")
    releases = previous["releases"] if previous is not None else []
    if releases and version_key(item["version"]) <= version_key(releases[0]["version"]):
        raise Refusal(
            f"{item['version']} is not higher than {releases[0]['version']}, "
            "the newest release the feed already carries"
        )
    payload = {
        "format": FORMAT,
        "product": PRODUCT,
        "generated_at": generated_at,
        "releases": [item, *releases][:MAX_RELEASES],
    }
    check_payload(payload, "the merged feed")
    write_file(arguments.out, compact(payload))
    count = len(payload["releases"])
    print(f"update_feed: {arguments.out}, {count} releases, newest {item['version']}")


def run_sign(arguments):
    payload = read_bytes(arguments.payload, "the payload", MAX_FEED_BYTES)
    check_payload(decode_json(payload, "the payload"), f"the payload at {arguments.payload}")
    keys = read_keys(arguments.keys)
    key_id, public = current_key(keys)
    signature = sign_bytes(payload)
    if not signature_verifies(public, payload, signature):
        raise Refusal(
            f"the key on standard input is not {key_id}'s: its signature does not verify "
            f"against {key_id}'s public key in {arguments.keys}"
        )
    write_file(arguments.out, envelope_bytes(key_id, payload, signature))
    print(f"update_feed: {arguments.out}, signed by {key_id}")


def run_verify(arguments):
    keys = read_keys(arguments.keys)
    what = f"the feed at {arguments.feed}"
    data = read_bytes(arguments.feed, "the feed", MAX_FEED_BYTES)
    key_id, _, payload = open_envelope(data, keys, what)
    if arguments.decoded is not None:
        write_file(arguments.decoded, pretty({"key_id": key_id, "payload": payload}))
    releases = payload["releases"]
    print(
        f"update_feed: {arguments.feed} verifies against {key_id}: {len(releases)} releases, "
        f"newest {releases[0]['version']}, generated {payload['generated_at']}"
    )


def parse_arguments(argv):
    parser = Parser(prog="update_feed.py", add_help=False, allow_abbrev=False)
    commands = parser.add_subparsers(dest="command", required=True, parser_class=Parser)
    flags = {
        "item": ("packages", "version", "pin", "notes", "base-url", "published-at", "out"),
        "merge": ("item", "keys", "generated-at", "out"),
        "sign": ("payload", "keys", "out"),
        "verify": ("feed", "keys"),
    }
    runs = {"item": run_item, "merge": run_merge, "sign": run_sign, "verify": run_verify}
    for command, names in flags.items():
        sub = commands.add_parser(command, add_help=False, allow_abbrev=False)
        for name in names:
            sub.add_argument(f"--{name}", required=True)
        sub.set_defaults(run=runs[command])
    commands.choices["merge"].add_argument("--previous")
    commands.choices["merge"].add_argument("--floor")
    commands.choices["verify"].add_argument("--decoded")
    return parser.parse_args(argv)


def main(argv):
    try:
        arguments = parse_arguments(argv)
        arguments.run(arguments)
    except Refusal as refusal:
        print(f"update_feed: {refusal}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
