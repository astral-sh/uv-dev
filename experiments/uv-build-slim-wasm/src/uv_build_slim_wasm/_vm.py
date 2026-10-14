"""Execution support for generated Python basic blocks; no opcode interpreter."""

import hashlib
import json
import math
import struct
import zlib
from collections import OrderedDict
from pathlib import Path
from threading import RLock
from types import SimpleNamespace


class Trap(RuntimeError):
    pass


def _take(stack, count):
    if not count:
        return []
    values = stack[-count:]
    del stack[-count:]
    return values


def _branch(stack, height, arity):
    results = _take(stack, arity)
    del stack[height:]
    stack.extend(results)


def _signed(value, bits):
    return value - (1 << bits) if value & (1 << (bits - 1)) else value


def _rotate(value, amount, bits, left):
    amount %= bits
    if not left:
        amount = (-amount) % bits
    return ((value << amount) | (value >> ((-amount) % bits))) & ((1 << bits) - 1)


def _integer_div(left, right, bits, signed, remainder):
    if not right:
        raise Trap("integer divide by zero")
    if signed:
        left, right = _signed(left, bits), _signed(right, bits)
        if not remainder and left == -(1 << (bits - 1)) and right == -1:
            raise Trap("integer overflow")
        quotient = abs(left) // abs(right)
        if (left < 0) != (right < 0):
            quotient = -quotient
    else:
        quotient = left // right
    return (left - quotient * right if remainder else quotient) & ((1 << bits) - 1)


def _float_bits(value, bits):
    return struct.unpack(
        "<f" if bits == 32 else "<d", value.to_bytes(bits // 8, "little")
    )[0]


def _int_bits(value, bits):
    return int.from_bytes(struct.pack("<f" if bits == 32 else "<d", value), "little")


def _round_float(value, bits):
    if bits == 64:
        return value
    try:
        return struct.unpack("<f", struct.pack("<f", value))[0]
    except OverflowError:
        return math.copysign(math.inf, value)


def _trunc(value):
    if not math.isfinite(value) or value == 0:
        return value
    return math.copysign(float(math.trunc(value)), value)


def _float_div(left, right):
    if right == 0:
        if left == 0 or math.isnan(left):
            return math.nan
        return math.copysign(math.inf, math.copysign(1, left) * math.copysign(1, right))
    return left / right


class Memory:
    def __init__(self, minimum, maximum, address_bits=32):
        self.address_bits = address_bits
        self.size = minimum
        self.maximum = (
            (maximum or 65536)
            if address_bits == 32
            else ((1 << 48) if maximum is None else maximum)
        )
        self.data = bytearray(minimum * 65536)
        self.external = None

    def bounds(self, address, size):
        if self.external is not None and address >= len(self.data):
            self.external.validate_memory(address, size)
            return
        if address < 0 or size < 0 or address + size > len(self.data):
            raise Trap("out of bounds memory access")

    def get(self, address, size):
        if self.external is not None and address >= len(self.data):
            return self.external.read_memory(address, size)
        self.bounds(address, size)
        return self.data[address : address + size]

    def put(self, address, data):
        if self.external is not None and address >= len(self.data):
            self.external.write_memory(address, data)
            return
        self.bounds(address, len(data))
        self.data[address : address + len(data)] = data

    def get_u8(self, address):
        return int.from_bytes(self.get(address, 1), "little")

    def get_u16(self, address):
        return int.from_bytes(self.get(address, 2), "little")

    def get_u32(self, address):
        return int.from_bytes(self.get(address, 4), "little")

    def get_u64(self, address):
        return int.from_bytes(self.get(address, 8), "little")

    def get_i32(self, address):
        return _signed(self.get_u32(address), 32)

    def get_i64(self, address):
        return _signed(self.get_u64(address), 64)

    def put_u8(self, address, value):
        self.put(address, (value & 255).to_bytes(1, "little"))

    def put_u16(self, address, value):
        self.put(address, (value & 65535).to_bytes(2, "little"))

    def put_u32(self, address, value):
        self.put(address, (value & 4294967295).to_bytes(4, "little"))

    def put_u64(self, address, value):
        self.put(address, (value & 18446744073709551615).to_bytes(8, "little"))

    put_i32 = put_u32
    put_i64 = put_u64


class Program:
    def __init__(self, directory=None, package=__package__):
        if directory is None:
            from importlib import resources

            def read(name):
                if hasattr(resources, "files"):
                    return resources.files(package).joinpath(name).read_bytes()
                return resources.read_binary(package, name)
        else:
            directory = Path(directory)

            def read(name):
                return (directory / name).read_bytes()

        self.description = json.loads(read("_program.json"))
        self.code = read("_code.bin")
        compressed_data = read("_data.bin")
        if (
            "data_sha256" in self.description
            and hashlib.sha256(compressed_data).hexdigest()
            != self.description["data_sha256"]
        ):
            raise ValueError("Generated data checksum mismatch")
        self.data = zlib.decompress(compressed_data)
        if hashlib.sha256(self.code).hexdigest() != self.description["code_sha256"]:
            raise ValueError("Generated Python program checksum mismatch")
        self.compiled = {}
        self.chunks = OrderedDict()
        self._compile_lock = RLock()

    def function(self, index):
        if index in self.compiled:
            return self.compiled[index]
        # Compilation and chunk eviction form one operation. Independent build
        # machines share this cache, including on free-threaded Python builds.
        with self._compile_lock:
            return self._compile_function(index)

    def _compile_function(self, index):
        if index not in self.compiled:
            description = self.description["functions"][index]
            if "chunks" in self.description:
                chunk_index = description["chunk"]
                if chunk_index not in self.chunks:
                    chunk = self.description["chunks"][chunk_index]
                    data = zlib.decompress(
                        self.code[chunk["offset"] : chunk["offset"] + chunk["length"]]
                    )
                    if len(data) != chunk["raw_length"]:
                        raise ValueError("Generated Python chunk length mismatch")
                    self.chunks[chunk_index] = data
                    if len(self.chunks) > 8:
                        self.chunks.popitem(last=False)
                self.chunks.move_to_end(chunk_index)
                source = self.chunks[chunk_index][
                    description["offset"] : description["offset"]
                    + description["length"]
                ]
            else:
                source = zlib.decompress(
                    self.code[
                        description["offset"] : description["offset"]
                        + description["length"]
                    ]
                )
            if hashlib.sha256(source).hexdigest() != description["source_sha256"]:
                raise ValueError("Generated Python function checksum mismatch")
            namespace = {
                name: value for name, value in globals().items() if name.startswith("_")
            }
            namespace["Trap"] = Trap
            exec(
                compile(source, "<generated-rust-function-%d>" % index, "exec"),
                namespace,
            )
            self.compiled[index] = namespace["BLOCKS"]
        return self.compiled[index]


class Machine:
    def __init__(self, program, host):
        self.program, self.host = program, host
        self.description = program.description
        self.memory = Memory(*self.description["memory"])
        self.globals = list(self.description["globals"])
        self.tables = [list(table) for table in self.description["tables"]]
        for address, offset, size in self.description["segments"]:
            self.memory.put(address, program.data[offset : offset + size])
        self.host_context = SimpleNamespace(
            store=SimpleNamespace(mems=[self.memory]),
            stack=SimpleNamespace(
                frame=[SimpleNamespace(module=SimpleNamespace(mems=[0]))]
            ),
        )
        if hasattr(host, "bind"):
            host.bind(self)

    def indirect(self, table, index, expected_type):
        if index >= len(self.tables[table]):
            if hasattr(self.host, "indirect"):
                return self.host.indirect(index, expected_type)
            raise Trap("undefined table element")
        target = self.tables[table][index]
        if target is None:
            raise Trap("uninitialized table element")
        actual = self.description["function_types"][target]
        if (
            self.description["types"][actual]
            != self.description["types"][expected_type]
        ):
            raise Trap("indirect call type mismatch")
        return target

    def load(self, address, width, signed, floating, bits):
        data = self.memory.get(address, width)
        if floating:
            return struct.unpack("<f" if width == 4 else "<d", data)[0]
        return int.from_bytes(data, "little", signed=signed) & ((1 << bits) - 1)

    def store(self, address, value, width, floating):
        data = (
            struct.pack("<f" if width == 4 else "<d", value)
            if floating
            else (value & ((1 << (width * 8)) - 1)).to_bytes(width, "little")
        )
        self.memory.put(address, data)

    def copy(self, destination, source, size):
        self.memory.put(destination, self.memory.get(source, size))

    def fill(self, destination, value, size):
        self.memory.bounds(destination, size)
        self.memory.put(destination, bytes([value & 255]) * size)

    def grow(self, pages):
        if self.memory.size + pages > self.memory.maximum:
            return (1 << self.memory.address_bits) - 1
        previous = self.memory.size
        try:
            self.memory.data.extend(bytearray(pages * 65536))
        except (MemoryError, OverflowError):
            return (1 << self.memory.address_bits) - 1
        self.memory.size += pages
        return previous

    def call(self, index, arguments):
        imports = self.description["imports"]

        def frame(function, args):
            body = function - len(imports)
            description = self.description["functions"][body]
            return [
                self.program.function(body),
                list(args) + description["zeros"],
                [],
                [0] * description["labels"],
                0,
            ]

        frames = [frame(index, arguments)]
        while frames:
            current = frames[-1]
            blocks, locals_, stack, bases, pc = current
            action = blocks[pc](self, locals_, stack, bases)
            if isinstance(action, int):
                current[4] = action
                continue
            if action[0] == "return":
                results = _take(stack, action[1])
                frames.pop()
                if not frames:
                    return results
                frames[-1][2].extend(results)
                continue
            _, target, values, resume = action
            current[4] = resume
            if isinstance(target, tuple) or target < len(imports):
                if isinstance(target, tuple):
                    module, name, type_index = target
                else:
                    module, name, type_index = imports[target]
                types, result_types = self.description["types"][type_index]
                host_arguments = [
                    _signed(value, 32 if kind == 127 else 64)
                    if kind in {126, 127}
                    else value
                    for kind, value in zip(types, values)
                ]
                result = self.host(module, name, self.host_context, host_arguments)
                stack.extend(
                    value & ((1 << (32 if kind == 127 else 64)) - 1)
                    if kind in {126, 127}
                    else value
                    for kind, value in zip(result_types, result or [])
                )
            else:
                frames.append(frame(target, values))
        return []

    def start(self):
        entry = self.description.get("entry")
        if entry is None:
            return self.call(self.description["exports"]["_start"][1], [])
        for name in entry["initializers"]:
            self.call(self.description["exports"][name][1], [])
        result = self.call(
            self.description["exports"][entry["name"]][1], entry["arguments"]
        )
        if entry["return_status"] and result[0]:
            raise Exception(SystemExit, _signed(result[0], 32))
        return result
