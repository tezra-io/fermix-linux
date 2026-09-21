#!/usr/bin/env python3
"""What an ELF file says about the libraries it needs and where it looks.

One reader, used by the two gates that ask an ELF a question:
`scripts/check_private_runtime.sh`, which holds every object in the private
prefix to the search path the design gives it, and
`scripts/package_dependencies.py`, which proves that nothing in the package
needs a library the package neither carries nor declares.

It reads the file itself rather than shelling out to `readelf` or `objdump`.
Two reasons, and neither is taste. The build container is AlmaLinux and the
gates also run outside it, so a binutils that is present in one place and absent
in another would make a gate that passes by not running. And the answer these
gates need is four fields of the dynamic section, which is less code to read
than it is to parse out of another tool's output in two formats.

What it answers, per file:

  needed        the DT_NEEDED entries, in order
  runpath       the DT_RUNPATH string, or None
  rpath         the DT_RPATH string, or None, which is the older tag and is a
                refusal wherever a RUNPATH was asked for
  soname        the DT_SONAME string, or None
  interpreter   the PT_INTERP path, or None. The engine's executables name a
                musl loader under /var/lib/fermix, which is how they are told
                apart from everything else in the package
  glibc         the highest GLIBC_x.y version any symbol requires, as a tuple,
                or None. This is the floor the package must declare
  machine       the e_machine value, so a tree for the wrong architecture is a
                refusal rather than a puzzle

Usage as a program, which is what the shell gates use:
  elf_facts.py <path>...        one JSON object per line, keyed by path
  elf_facts.py --tree <dir>     every ELF under the tree, the same way
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
from pathlib import Path

ELF_MAGIC = b"\x7fELF"

PT_INTERP = 3
PT_DYNAMIC = 2

DT_NULL = 0
DT_NEEDED = 1
DT_STRTAB = 5
DT_SONAME = 14
DT_RPATH = 15
DT_RUNPATH = 29

SHT_GNU_VERNEED = 0x6FFFFFFE


class NotAnElf(Exception):
    """The file is not an ELF at all, which is not an error in a tree walk."""


class Malformed(Exception):
    """The file claims to be an ELF and does not hold together."""


class Elf:
    """Just enough of an ELF file to answer the four questions above."""

    def __init__(self, data: bytes, path: str):
        self.path = path
        self.data = data
        if len(data) < 64 or data[:4] != ELF_MAGIC:
            raise NotAnElf(path)
        self.bits = {1: 32, 2: 64}.get(data[4])
        if self.bits is None:
            raise Malformed(f"{path}: unknown ELF class {data[4]}")
        endian = data[5]
        if endian not in (1, 2):
            raise Malformed(f"{path}: unknown ELF endianness {endian}")
        self.order = "<" if endian == 1 else ">"
        self.machine = self._u16(18)
        self._read_program_headers()
        self._read_section_headers()

    # ---- the primitive reads ------------------------------------------------

    def _at(self, offset: int, size: int) -> bytes:
        chunk = self.data[offset : offset + size]
        if len(chunk) != size:
            raise Malformed(f"{self.path}: truncated at offset {offset}")
        return chunk

    def _u16(self, offset: int) -> int:
        return struct.unpack(self.order + "H", self._at(offset, 2))[0]

    def _u32(self, offset: int) -> int:
        return struct.unpack(self.order + "I", self._at(offset, 4))[0]

    def _u64(self, offset: int) -> int:
        return struct.unpack(self.order + "Q", self._at(offset, 8))[0]

    def _word(self, offset: int) -> int:
        return self._u64(offset) if self.bits == 64 else self._u32(offset)

    def _string(self, offset: int) -> str:
        end = self.data.find(b"\0", offset)
        if end < 0:
            raise Malformed(f"{self.path}: unterminated string at {offset}")
        return self.data[offset:end].decode("utf-8", "replace")

    # ---- the tables ---------------------------------------------------------

    def _read_program_headers(self) -> None:
        if self.bits == 64:
            offset, entry_size, count = self._u64(32), self._u16(54), self._u16(56)
        else:
            offset, entry_size, count = self._u32(28), self._u16(42), self._u16(44)
        self.segments = []
        for index in range(count):
            base = offset + index * entry_size
            kind = self._u32(base)
            if self.bits == 64:
                file_offset, size = self._u64(base + 8), self._u64(base + 32)
            else:
                file_offset, size = self._u32(base + 4), self._u32(base + 16)
            self.segments.append((kind, file_offset, size))

    def _read_section_headers(self) -> None:
        if self.bits == 64:
            offset, entry_size, count = self._u64(40), self._u16(58), self._u16(60)
        else:
            offset, entry_size, count = self._u32(32), self._u16(46), self._u16(48)
        self.sections = []
        for index in range(count):
            base = offset + index * entry_size
            kind = self._u32(base + 4)
            if self.bits == 64:
                file_offset = self._u64(base + 24)
                size = self._u64(base + 32)
                link = self._u32(base + 40)
                info = self._u32(base + 44)
            else:
                file_offset = self._u32(base + 16)
                size = self._u32(base + 20)
                link = self._u32(base + 24)
                info = self._u32(base + 28)
            self.sections.append((kind, file_offset, size, link, info))

    def _section_offset(self, index: int) -> int:
        if index >= len(self.sections):
            raise Malformed(f"{self.path}: section {index} is past the table")
        return self.sections[index][1]

    # ---- the answers --------------------------------------------------------

    @property
    def interpreter(self) -> str | None:
        for kind, offset, size in self.segments:
            if kind == PT_INTERP and size:
                return self._string(offset)
        return None

    def _dynamic_entries(self) -> list[tuple[int, int]]:
        for kind, offset, size in self.segments:
            if kind != PT_DYNAMIC:
                continue
            step = 16 if self.bits == 64 else 8
            entries = []
            for position in range(offset, offset + size, step):
                tag = self._word(position)
                value = self._word(position + step // 2)
                if tag == DT_NULL:
                    break
                entries.append((tag, value))
            return entries
        return []

    def _string_table(self, entries: list[tuple[int, int]]) -> int | None:
        """The file offset of DT_STRTAB, whose value is a virtual address."""
        # DT_STRTAB carries a virtual address, and every reader of the dynamic
        # section has to map it back to a file offset through the loadable
        # segments, which is the mapping the loader itself would use.
        for tag, value in entries:
            if tag == DT_STRTAB:
                return self._address_to_offset(value)
        return None

    def _address_to_offset(self, address: int) -> int | None:
        PT_LOAD = 1
        if self.bits == 64:
            unpack = self._u64
            fields = (8, 16, 32)
        else:
            unpack = self._u32
            fields = (4, 8, 16)
        header_offset, entry_size, count = (
            (self._u64(32), self._u16(54), self._u16(56))
            if self.bits == 64
            else (self._u32(28), self._u16(42), self._u16(44))
        )
        for index in range(count):
            base = header_offset + index * entry_size
            if self._u32(base) != PT_LOAD:
                continue
            file_offset = unpack(base + fields[0])
            virtual = unpack(base + fields[1])
            size = unpack(base + fields[2])
            if virtual <= address < virtual + size:
                return file_offset + (address - virtual)
        return None

    def facts(self) -> dict:
        entries = self._dynamic_entries()
        table = self._string_table(entries)

        def text(value: int) -> str | None:
            if table is None:
                return None
            return self._string(table + value)

        needed: list[str] = []
        runpath = rpath = soname = None
        for tag, value in entries:
            if tag == DT_NEEDED:
                name = text(value)
                if name:
                    needed.append(name)
            elif tag == DT_RUNPATH:
                runpath = text(value)
            elif tag == DT_RPATH:
                rpath = text(value)
            elif tag == DT_SONAME:
                soname = text(value)

        return {
            "needed": needed,
            "runpath": runpath,
            "rpath": rpath,
            "soname": soname,
            "interpreter": self.interpreter,
            "glibc": self.required_glibc(),
            "machine": self.machine,
            "dynamic": bool(entries),
        }

    def required_glibc(self) -> list[int] | None:
        """The highest GLIBC_x.y any symbol in this object requires.

        Read out of .gnu.version_r, which is where the linker records, per
        library, the exact version names the object's undefined symbols were
        bound to. It is the only honest source for "what glibc does this need":
        the NEEDED entry names `libc.so.6` on every host that ever existed.
        """
        highest: tuple[int, ...] | None = None
        for kind, offset, size, link, info in self.sections:
            if kind != SHT_GNU_VERNEED:
                continue
            strings = self._section_offset(link)
            position = offset
            for _ in range(info):
                count = self._u16(position + 2)
                aux = self._u32(position + 8)
                next_entry = self._u32(position + 12)
                aux_position = position + aux
                for _ in range(count):
                    name = self._string(strings + self._u32(aux_position + 8))
                    parsed = parse_glibc_version(name)
                    if parsed and (highest is None or parsed > highest):
                        highest = parsed
                    step = self._u32(aux_position + 12)
                    if not step:
                        break
                    aux_position += step
                if not next_entry:
                    break
                position += next_entry
        return list(highest) if highest else None


def parse_glibc_version(name: str) -> tuple[int, ...] | None:
    """`GLIBC_2.34` as (2, 34). Anything else, including GLIBC_PRIVATE, is None."""
    if not name.startswith("GLIBC_"):
        return None
    rest = name[len("GLIBC_") :]
    pieces = rest.split(".")
    if not all(piece.isdigit() for piece in pieces) or not pieces:
        return None
    return tuple(int(piece) for piece in pieces)


def read(path: str | os.PathLike) -> dict | None:
    """The facts for one file, or None when it is not an ELF."""
    try:
        with open(path, "rb") as handle:
            data = handle.read()
    except OSError as error:
        raise Malformed(f"{path}: {error}") from error
    try:
        return Elf(data, str(path)).facts()
    except NotAnElf:
        return None


def walk(tree: str | os.PathLike) -> list[tuple[str, dict]]:
    """Every ELF under a tree, by path, with symbolic links skipped.

    A link is skipped because its target is walked in its own right, and
    counting it twice would report one object's RUNPATH under two names.
    """
    found = []
    for directory, subdirectories, names in os.walk(tree):
        subdirectories.sort()
        for name in sorted(names):
            path = os.path.join(directory, name)
            if os.path.islink(path) or not os.path.isfile(path):
                continue
            facts = read(path)
            if facts is not None:
                found.append((path, facts))
    return found


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", type=Path)
    parser.add_argument("--tree", type=Path, help="every ELF under this directory")
    arguments = parser.parse_args(argv)

    if not arguments.paths and arguments.tree is None:
        parser.error("name a file or pass --tree <dir>")

    try:
        rows: list[tuple[str, dict]] = []
        if arguments.tree is not None:
            if not arguments.tree.is_dir():
                raise Malformed(f"no tree at {arguments.tree}")
            rows.extend(walk(arguments.tree))
        for path in arguments.paths:
            facts = read(path)
            if facts is None:
                raise Malformed(f"{path} is not an ELF file")
            rows.append((str(path), facts))
    except Malformed as error:
        print(f"elf_facts: {error}", file=sys.stderr)
        return 1

    for path, facts in rows:
        print(json.dumps({"path": path, **facts}, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
