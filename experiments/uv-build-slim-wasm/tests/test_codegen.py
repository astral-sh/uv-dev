"""Compare generated arithmetic, control flow, and memory with V8 WebAssembly."""

import argparse
import json
import math
import os
import random
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))
from uv_build_slim_wasm._vm import Machine, Program, Trap


def uleb(value):
    result = bytearray()
    while True:
        byte = value & 127
        value >>= 7
        result.append(byte | (128 if value else 0))
        if not value:
            return bytes(result)


def vector(values):
    return uleb(len(values)) + b"".join(values)


def section(kind, data):
    return bytes([kind]) + uleb(len(data)) + data


def module(memory64=False):
    functions = []
    types = {"i32": 127, "i64": 126, "f32": 125, "f64": 124}
    address_type = "i64" if memory64 else "i32"
    address_const = b"\x42" if memory64 else b"\x41"

    def add(name, arguments, result, body, locals_=b"\x00"):
        functions.append((name, arguments, result, locals_ + body + b"\x0b"))

    for kind, first in [("i32", 0x6A), ("i64", 0x7C)]:
        for offset, operation in enumerate(
            [
                "add",
                "sub",
                "mul",
                "div_s",
                "div_u",
                "rem_s",
                "rem_u",
                "and",
                "or",
                "xor",
                "shl",
                "shr_s",
                "shr_u",
                "rotl",
                "rotr",
            ]
        ):
            add(
                kind + "." + operation,
                [kind, kind],
                kind,
                b"\x20\x00\x20\x01" + bytes([first + offset]),
            )
        for offset, operation in enumerate(["clz", "ctz", "popcnt"]):
            add(
                kind + "." + operation,
                [kind],
                kind,
                b"\x20\x00" + bytes([first - 3 + offset]),
            )
    for kind, first in [("f32", 0x92), ("f64", 0xA0)]:
        for offset, operation in enumerate(["add", "sub", "mul", "div"]):
            add(
                kind + "." + operation,
                [kind, kind],
                kind,
                b"\x20\x00\x20\x01" + bytes([first + offset]),
            )
    add(
        "if_result",
        ["i32", "i32", "i32"],
        "i32",
        b"\x20\x02\x04\x7f\x20\x00\x20\x01\x6a\x05\x20\x00\x20\x01\x6b\x0b",
    )
    add(
        "sum_loop",
        ["i32"],
        "i32",
        b"\x41\x00\x21\x01\x02\x40\x03\x40\x20\x00\x45\x0d\x01\x20\x01\x20\x00\x6a\x21\x01\x20\x00\x41\x01\x6b\x21\x00\x0c\x00\x0b\x0b\x20\x01",
        b"\x01\x01\x7f",
    )
    add(
        "br_table",
        ["i32"],
        "i32",
        b"\x02\x7f\x02\x7f\x41\x0a\x20\x00\x0e\x01\x00\x01\x0b\x41\x01\x6a\x0b",
    )
    add("direct_call", ["i32", "i32"], "i32", b"\x20\x00\x20\x01\x10\x00")
    add(
        "indirect_call",
        ["i32", "i32", "i32"],
        "i32",
        b"\x20\x00\x20\x01\x20\x02\x11\x00\x00",
    )
    add(
        "memory_overlap",
        ["i32"],
        "i32",
        address_const
        + b"\x00\x20\x00\x36\x02\x00"
        + address_const
        + b"\x01"
        + address_const
        + b"\x00"
        + address_const
        + b"\x04\xfc\x0a\x00\x00"
        + address_const
        + b"\x01\x28\x02\x00",
    )
    add(
        "memory_fill",
        ["i32"],
        "i32",
        address_const
        + b"\x00\x20\x00"
        + address_const
        + b"\x04\xfc\x0b\x00"
        + address_const
        + b"\x00\x28\x02\x00",
    )
    add("memory_bounds", [address_type], "i32", b"\x20\x00\x28\x02\x00")
    add("memory_grow", [address_type], address_type, b"\x20\x00\x40\x00")
    add("global", ["i32"], "i32", b"\x20\x00\x24\x00\x23\x00")
    add("select", ["i64", "i64", "i32"], "i64", b"\x20\x00\x20\x01\x20\x02\x1b")
    # Typed if blocks consume parameters before either branch and restore their
    # result at the original stack height. Type 0 is (i32, i32) -> i32.
    add(
        "typed_if",
        ["i32", "i32", "i32"],
        "i32",
        b"\x20\x00\x20\x01\x20\x02\x04\x00\x6a\x05\x6b\x0b",
    )
    type_section = vector(
        [
            b"\x60"
            + vector([bytes([types[arg]]) for arg in args])
            + b"\x01"
            + bytes([types[result]])
            for _, args, result, _ in functions
        ]
    )
    function_section = vector([uleb(index) for index in range(len(functions))])
    exports = vector(
        [
            uleb(len(name)) + name.encode() + b"\x00" + uleb(index)
            for index, (name, _, _, _) in enumerate(functions)
        ]
    )
    code = vector([uleb(len(body)) + body for _, _, _, body in functions])
    data = (
        b"\x00asm\x01\x00\x00\x00"
        + section(1, type_section)
        + section(3, function_section)
    )
    data += section(4, b"\x01\x70\x00\x02") + section(
        5, b"\x01" + bytes([5 if memory64 else 1]) + b"\x01\x02"
    )
    data += section(6, b"\x01\x7f\x01\x41\x00\x0b") + section(7, exports)
    data += section(9, b"\x01\x00\x41\x00\x0b\x02\x00\x01") + section(10, code)
    return data, functions


NODE = r"""
const fs = require('fs');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const module_ = new WebAssembly.Module(fs.readFileSync(process.argv[1]));
function encode(value, type) {
  if (type === 'i64') return BigInt.asUintN(64, value).toString();
  if (type === 'i32') return (value >>> 0).toString();
  if (Number.isNaN(value)) return 'nan';
  const buffer = Buffer.alloc(type === 'f32' ? 4 : 8);
  if (type === 'f32') buffer.writeFloatLE(value); else buffer.writeDoubleLE(value);
  return buffer.toString('hex');
}
const results = input.map(test => {
  const instance = new WebAssembly.Instance(module_);
  const args = test.args.map((value, index) => test.types[index] === 'i64' ? BigInt(value) : value === 'inf' ? Infinity : value === '-inf' ? -Infinity : Number(value));
  try { return encode(instance.exports[test.name](...args), test.result); }
  catch (error) { if (error instanceof WebAssembly.RuntimeError) return 'trap'; throw error; }
});
process.stdout.write(JSON.stringify(results));
"""


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pywasm", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--memory64", action="store_true")
    args = parser.parse_args()
    binary, functions = module(args.memory64)
    rng = random.Random(452938)
    cases = []
    for name, types, result, _ in functions:
        if name.startswith(("i32.", "i64.")):
            bits = int(name[1:3])
            boundary = [
                0,
                1,
                2,
                bits - 1,
                bits,
                bits + 1,
                (1 << (bits - 1)) - 1,
                1 << (bits - 1),
                (1 << bits) - 1,
            ]
            values = [(a, b) for a in boundary for b in boundary] + [
                (rng.getrandbits(bits), rng.getrandbits(bits)) for _ in range(30)
            ]
            arguments = [pair[: len(types)] for pair in values]
        elif name.startswith(("f32.", "f64.")):
            values = [
                0.0,
                -0.0,
                1.0,
                -1.0,
                1e-30,
                1e30,
                math.inf,
                -math.inf,
                math.nan,
                16777217.0,
                1.23456789,
            ]
            arguments = [(a, b) for a in values for b in values]
        elif name in {"if_result", "typed_if", "select"}:
            arguments = [(11, 3, condition) for condition in [0, 1, 2, 0xFFFFFFFF]]
        elif name == "sum_loop":
            arguments = [(value,) for value in [0, 1, 2, 31, 100]]
        elif name == "indirect_call":
            arguments = [(11, 3, index) for index in [0, 1, 2, 0xFFFFFFFF]]
        elif name == "direct_call":
            arguments = [(11, 3)]
        elif name == "memory_bounds":
            arguments = [(value,) for value in [0, 65532, 65533, 65536, 0xFFFFFFFF]]
            if args.memory64:
                arguments += [(1 << 32,), (1 << 63,), ((1 << 64) - 1,)]
        elif name == "memory_grow":
            arguments = [(value,) for value in [0, 1, 2, 65536]]
            if args.memory64:
                arguments += [(1 << 32,), (1 << 63,), ((1 << 64) - 1,)]
        else:
            arguments = [(value,) for value in [0, 1, 2, 0x12345678, 0xFFFFFFFF]]
        for values in arguments:
            cases.append(
                {
                    "name": name,
                    "types": types,
                    "result": result,
                    "args": [str(value) for value in values],
                }
            )
    scratch = Path(
        os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
    )
    with tempfile.TemporaryDirectory(dir=str(scratch)) as temporary:
        root = Path(temporary)
        wasm = root / "operations.wasm"
        wasm.write_bytes(binary)
        subprocess.run(
            [
                sys.executable,
                str(ROOT / "codegen/generate.py"),
                "--wasm",
                str(wasm),
                "--pywasm",
                str(args.pywasm),
                "--output",
                str(root / "program"),
            ],
            check=True,
        )
        reference = json.loads(
            subprocess.check_output(
                [
                    "node",
                    *(["--experimental-wasm-memory64"] if args.memory64 else []),
                    "-e",
                    NODE,
                    str(wasm),
                ],
                input=json.dumps(cases),
                text=True,
            )
        )
        program = Program(root / "program")
        for case, expected in zip(cases, reference):
            machine = Machine(program, None)
            values = []
            for text, kind in zip(case["args"], case["types"]):
                value = int(text) if kind.startswith("i") else float(text)
                if kind == "f32":
                    value = struct.unpack("<f", struct.pack("<f", value))[0]
                values.append(value)
            try:
                result = machine.call(
                    program.description["exports"][case["name"]][1], values
                )[0]
                if case["result"].startswith("i"):
                    actual = str(result)
                elif math.isnan(result):
                    actual = "nan"
                else:
                    actual = struct.pack(
                        "<f" if case["result"] == "f32" else "<d", result
                    ).hex()
            except Trap:
                actual = "trap"
            assert actual == expected, (case, expected, actual)
    report = {
        "reference": subprocess.check_output(["node", "--version"], text=True).strip(),
        "functions": len(functions),
        "comparisons": len(cases),
        "memory_address_bits": 64 if args.memory64 else 32,
        "equal": True,
    }
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
