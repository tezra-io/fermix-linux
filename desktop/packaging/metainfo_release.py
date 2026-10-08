#!/usr/bin/env python3
"""Render the package's one <release> into the installed copy of the AppStream metainfo.

    metainfo_release.py --metainfo <io.tezra.Fermix.metainfo.xml> --version <package version>
                        --date <YYYY-MM-DD> --out <file>

The repository's metainfo carries no <releases>: the version is the package's, known only when a
package is built. This adds <releases> with one <release version="<version>" date="<date>"/> just
before </component>, and changes nothing else, so GNOME Software and KDE Discover show the version
the package installs. The date is the UTC date of the build's timestamp.
"""

import argparse
import datetime
import os
import re
import sys
from xml.etree import ElementTree

VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(\+[0-9A-Za-z.]+)?")
DATE = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}")
CLOSING = "</component>"
USAGE = ("metainfo_release.py --metainfo <file> --version <package version> --date <YYYY-MM-DD> "
         "--out <file>")  # fmt: skip


class Refusal(Exception):
    """A refusal; its message is the sentence printed after "metainfo_release: "."""


class Parser(argparse.ArgumentParser):
    """argparse, refusing a malformed command line the way every other mistake here is refused."""

    def error(self, message):
        raise Refusal(f"usage: {USAGE} ({message})")


def check_version(version):
    if not VERSION.fullmatch(version):
        raise Refusal(f"{version} is not a package version: X.Y.Z, X.Y.Z+N or X.Y.Z+0.dev...")


def check_date(date):
    try:
        if not DATE.fullmatch(date):
            raise ValueError(date)
        datetime.date.fromisoformat(date)
    except ValueError as error:
        raise Refusal(f"{date} is not a date: YYYY-MM-DD") from error


def rendered(text, version, date, source):
    if "<releases" in text:
        raise Refusal(f"{source} already has <releases>; the package renders its own")
    if text.count(CLOSING) != 1:
        raise Refusal(f"{source} does not have one {CLOSING}")
    block = f'\n  <releases>\n    <release version="{version}" date="{date}"/>\n  </releases>\n'
    result = text.replace(CLOSING, block + CLOSING)
    try:
        releases = ElementTree.fromstring(result.encode()).findall("./releases/release")
    except ElementTree.ParseError as error:
        raise Refusal(f"{source} is not well-formed XML once rendered: {error}") from error
    if [r.attrib for r in releases] != [{"version": version, "date": date}]:
        raise Refusal(f"{source} does not hold the one release once rendered")
    return result


def read_text(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return handle.read()
    except OSError as error:
        raise Refusal(f"cannot read {path}: {error.strerror}") from error


def write(path, text):
    partial = f"{path}.partial"
    with open(partial, "w", encoding="utf-8") as handle:
        handle.write(text)
    os.replace(partial, path)


def parse_arguments(argv):
    parser = Parser(prog="metainfo_release.py", add_help=False)
    for name in ("metainfo", "version", "date", "out"):
        parser.add_argument(f"--{name}", required=True)
    return parser.parse_args(argv)


def main(argv):
    try:
        arguments = parse_arguments(argv)
        check_version(arguments.version)
        check_date(arguments.date)
        text = rendered(read_text(arguments.metainfo), arguments.version, arguments.date,
                        arguments.metainfo)  # fmt: skip
        write(arguments.out, text)
    except Refusal as refusal:
        print(f"metainfo_release: {refusal}", file=sys.stderr)
        return 1
    print(f"metainfo_release: {arguments.out}, release {arguments.version} of {arguments.date}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
