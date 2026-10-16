"""Read bounded ELF identity data without substituting filesystem-backed mappings."""

from __future__ import annotations

import struct
from pathlib import Path

MAX_METADATA = 2 * 1024 * 1024
MAX_HEADERS = 4096


class CaptureError(Exception):
    """An explicit reason to skip an unsupported or inconsistent capture."""


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise CaptureError(reason)


def header(data: bytes) -> dict:
    require(len(data) == 64 and data[:7] == b"\x7fELF\x02\x01\x01", "unsupported_elf")
    fields = struct.unpack("<16sHHIQQQIHHHHHH", data)
    require(fields[2] == 62 and fields[3] == 1, "unsupported_architecture")
    require(
        fields[9] == 56 and 0 < fields[10] <= MAX_HEADERS, "invalid_program_headers"
    )
    return {
        "type": fields[1],
        "entry": fields[4],
        "phoff": fields[5],
        "phnum": fields[10],
    }


def program_headers(data: bytes) -> list[dict]:
    require(len(data) % 56 == 0, "invalid_program_headers")
    return [
        dict(
            zip(
                (
                    "type",
                    "flags",
                    "offset",
                    "address",
                    "physical",
                    "filesz",
                    "memsz",
                    "align",
                ),
                values,
                strict=True,
            )
        )
        for values in struct.iter_unpack("<IIQQQQQQ", data)
    ]


def notes(data: bytes) -> list[tuple[bytes, int, bytes]]:
    require(len(data) <= MAX_METADATA, "oversized_notes")
    result = []
    offset = 0
    while offset < len(data):
        require(len(data) - offset >= 12, "truncated_note_header")
        namesz, descsz, kind = struct.unpack_from("<III", data, offset)
        require(0 < namesz <= 32 and descsz <= MAX_METADATA, "invalid_note_size")
        name_start = offset + 12
        desc_start = name_start + ((namesz + 3) & ~3)
        end = desc_start + ((descsz + 3) & ~3)
        require(end <= len(data), "truncated_note")
        result.append(
            (
                data[name_start : name_start + namesz].rstrip(b"\0"),
                kind,
                data[desc_start : desc_start + descsz],
            )
        )
        offset = end
    return result


def build_id(values: list[tuple[bytes, int, bytes]]) -> str:
    matches = [data for name, kind, data in values if name == b"GNU" and kind == 3]
    require(
        len(matches) == 1 and 4 <= len(matches[0]) <= 64,
        "missing_or_ambiguous_build_id",
    )
    return matches[0].hex()


class Elf:
    def __init__(self, path: Path):
        self.file = path.open("rb")
        self.size = path.stat().st_size
        self.header = header(self.read(0, 64))
        self.programs = program_headers(
            self.read(self.header["phoff"], self.header["phnum"] * 56)
        )

    def close(self) -> None:
        self.file.close()

    def read(self, offset: int, size: int) -> bytes:
        require(
            0 <= offset <= self.size and 0 <= size <= MAX_METADATA, "invalid_elf_read"
        )
        require(offset + size <= self.size, "truncated_elf")
        self.file.seek(offset)
        data = self.file.read(size)
        require(len(data) == size, "truncated_elf")
        return data

    def memory(self, address: int, size: int) -> bytes:
        # Only bytes physically present in a core `PT_LOAD` are usable here.
        matches = [
            item
            for item in self.programs
            if item["type"] == 1
            and item["address"] <= address
            and address + size <= item["address"] + item["filesz"]
        ]
        require(len(matches) == 1, "missing_or_ambiguous_core_bytes")
        item = matches[0]
        return self.read(item["offset"] + address - item["address"], size)

    def all_notes(self) -> list[tuple[bytes, int, bytes]]:
        values = []
        total = 0
        for item in self.programs:
            if item["type"] == 4:
                total += item["filesz"]
                require(total <= MAX_METADATA, "oversized_notes")
                values.extend(notes(self.read(item["offset"], item["filesz"])))
        return values


def executable_build_id(path: Path) -> str:
    elf = Elf(path)
    try:
        require(elf.header["type"] in (2, 3), "not_executable_elf")
        return build_id(elf.all_notes())
    finally:
        elf.close()


def core_identity(path: Path, executable: str) -> dict:
    elf = Elf(path)
    try:
        require(elf.header["type"] == 4, "not_core_elf")
        values = elf.all_notes()

        def one(kind: int) -> bytes:
            matches = [
                data
                for name, note_kind, data in values
                if name == b"CORE" and note_kind == kind
            ]
            require(len(matches) == 1, "missing_or_ambiguous_core_note")
            return matches[0]

        process = one(3)  # `NT_PRPSINFO`, Linux x86-64 layout.
        require(len(process) == 136, "unsupported_process_note")
        uid, gid, pid = struct.unpack_from("<IIi", process, 16)
        signal = one(0x53494749)  # `NT_SIGINFO`.
        require(len(signal) == 128, "unsupported_signal_note")
        signal_number = struct.unpack_from("<i", signal)[0]

        auxiliary = one(6)  # `NT_AUXV`; retain only main-image addresses.
        require(len(auxiliary) % 16 == 0, "invalid_auxiliary_note")
        addresses = {}
        for key, value in struct.iter_unpack("<QQ", auxiliary):
            if key in (3, 9):  # `AT_PHDR`, `AT_ENTRY`.
                require(key not in addresses, "ambiguous_auxiliary_note")
                addresses[key] = value
        require(addresses.keys() == {3, 9}, "missing_main_image_addresses")

        mappings = one(0x46494C45)  # `NT_FILE`.
        require(len(mappings) >= 16, "invalid_file_note")
        count, page_size = struct.unpack_from("<QQ", mappings)
        require(
            0 < count <= MAX_HEADERS
            and 0 < page_size <= 65536
            and page_size & (page_size - 1) == 0,
            "unsupported_file_note",
        )
        names_offset = 16 + count * 24
        require(names_offset < len(mappings), "truncated_file_note")
        names = mappings[names_offset:].split(b"\0")
        require(len(names) == count + 1 and names[-1] == b"", "invalid_mapping_names")
        matching = []
        for index, name in enumerate(names[:-1]):
            if name == executable.encode():
                start, end, offset = struct.unpack_from(
                    "<QQQ", mappings, 16 + index * 24
                )
                require(start < end, "invalid_mapping_range")
                matching.append((start, end, offset * page_size))
        roots = [item for item in matching if item[2] == 0]
        require(len(roots) == 1, "missing_or_ambiguous_main_mapping")
        start = roots[0][0]
        main = header(elf.memory(start, 64))
        require(main["type"] in (2, 3), "invalid_main_image")
        programs = program_headers(
            elf.memory(start + main["phoff"], main["phnum"] * 56)
        )
        first_load = [
            item for item in programs if item["type"] == 1 and item["offset"] == 0
        ]
        require(len(first_load) == 1, "ambiguous_main_load")
        bias = start - first_load[0]["address"]
        require(addresses[3] == start + main["phoff"], "main_phdr_mismatch")
        require(addresses[9] == bias + main["entry"], "main_entry_mismatch")
        require(
            any(low <= addresses[9] < high for low, high, _ in matching),
            "entry_outside_main_mapping",
        )
        require(
            any(
                item["type"] == 1
                and item["flags"] & 1
                and item["address"] <= main["entry"] < item["address"] + item["memsz"]
                for item in programs
            ),
            "entry_outside_executable_segment",
        )
        main_notes = []
        total = 0
        for item in programs:
            if item["type"] == 4:
                total += item["filesz"]
                require(total <= MAX_METADATA, "oversized_notes")
                main_notes.extend(
                    notes(elf.memory(bias + item["address"], item["filesz"]))
                )
        return {
            "pid": pid,
            "uid": uid,
            "gid": gid,
            "signal": signal_number,
            "build_id": build_id(main_notes),
        }
    finally:
        elf.close()
