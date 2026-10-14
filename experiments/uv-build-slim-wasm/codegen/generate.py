"""Lower an entire compiled WebAssembly module to Python basic blocks."""

import argparse
import collections
import hashlib
import json
import struct
import sys
import zlib
from pathlib import Path


class Generator:
    def __init__(self, module, opcode):
        self.module, self.opcode = module, opcode
        self.types = [
            ([v.data for v in t.args], [v.data for v in t.rets]) for t in module.type
        ]
        self.function_types = [entry.desc for entry in module.imps] + [
            f.type for f in module.func
        ]
        self.counts = collections.Counter()

    def signature(self, block):
        if block.kind == 0:
            return 0, 0
        if block.kind == 1:
            return 0, 1
        args, rets = self.types[block.data]
        return len(args), len(rets)

    def function(self, function):
        args, rets = self.types[function.type]
        nodes = []
        labels = [{"base": 0, "target": None, "arity": len(rets)}]

        def flatten(instructions, active):
            for instruction in instructions:
                op = self.opcode.name[instruction.opcode]
                values = instruction.args
                self.counts[op] += 1
                if op in {"block", "loop", "if"}:
                    n_args, n_rets = self.signature(values[0])
                    label = len(labels)
                    labels.append(
                        {
                            "base": label,
                            "target": None,
                            "arity": n_args if op == "loop" else n_rets,
                        }
                    )
                    nodes.append(
                        ("if_enter", [label, n_args, None])
                        if op == "if"
                        else ("enter", [label, n_args])
                    )
                    if op == "loop":
                        labels[label]["target"] = len(nodes)
                    if op == "if":
                        conditional = len(nodes) - 1
                        flatten(values[1], active + [label])
                        jump = len(nodes)
                        nodes.append(("jump", [None]))
                        nodes[conditional][1][2] = len(nodes)
                        flatten(values[2], active + [label])
                        nodes[jump][1][0] = len(nodes)
                    else:
                        flatten(values[1], active + [label])
                    nodes.append(("leave", [label, n_rets]))
                    if op != "loop":
                        labels[label]["target"] = len(nodes)
                elif op in {"br", "br_if"}:
                    nodes.append((op, [active[-1 - values[0]]]))
                elif op == "br_table":
                    nodes.append(
                        (
                            op,
                            [
                                [active[-1 - depth] for depth in values[0]],
                                active[-1 - values[1]],
                            ],
                        )
                    )
                else:
                    nodes.append((op, values))

        flatten(function.expr.data, [0])
        nodes.append(("return", []))
        labels[0]["target"] = len(nodes) - 1
        boundaries = {0}
        ending = {
            "br",
            "br_if",
            "br_table",
            "return",
            "unreachable",
            "call",
            "call_indirect",
            "jump",
            "if_enter",
        }
        for index, (op, values) in enumerate(nodes):
            if op in ending and index + 1 < len(nodes):
                boundaries.add(index + 1)
            if op == "jump":
                boundaries.add(values[0])
            if op == "if_enter":
                boundaries.add(values[2])
        boundaries.update(label["target"] for label in labels)
        boundaries = sorted(
            boundary for boundary in boundaries if boundary < len(nodes)
        )
        block_for = {start: index for index, start in enumerate(boundaries)}
        output = []
        for block, start in enumerate(boundaries):
            end = boundaries[block + 1] if block + 1 < len(boundaries) else len(nodes)
            output.append(f"def b{block}(v, L, S, B):")
            terminated = False
            for index in range(start, end):
                op, values = nodes[index]
                next_block = block_for.get(index + 1, block + 1)
                statements = []
                if op == "enter":
                    statements = [f"B[{values[0]}] = len(S) - {values[1]}"]
                elif op == "leave":
                    statements = [f"_branch(S, B[{values[0]}], {values[1]})"]
                elif op in {"br", "br_if"}:
                    label = labels[values[0]]
                    branch = f"_branch(S, B[{label['base']}], {label['arity']})"
                    target = block_for[label["target"]]
                    if op == "br":
                        statements = [branch, f"return {target}"]
                    else:
                        statements = [
                            "if S.pop():",
                            f"    {branch}",
                            f"    return {target}",
                            f"return {next_block}",
                        ]
                    terminated = True
                elif op == "br_table":
                    entries = [labels[label] for label in [*values[0], values[1]]]
                    table = tuple(
                        (entry["base"], entry["arity"], block_for[entry["target"]])
                        for entry in entries
                    )
                    statements = [
                        f"_table = {table!r}",
                        "_index = S.pop()",
                        "_base, _arity, _target = _table[_index if _index < len(_table)-1 else -1]",
                        "_branch(S, B[_base], _arity)",
                        "return _target",
                    ]
                    terminated = True
                elif op == "return":
                    statements = [f"return ('return', {len(rets)})"]
                    terminated = True
                elif op == "jump":
                    statements = [f"return {block_for[values[0]]}"]
                    terminated = True
                elif op == "if_enter":
                    statements = [
                        "_condition = S.pop()",
                        f"B[{values[0]}] = len(S)-{values[1]}",
                        f"return {next_block} if _condition else {block_for[values[2]]}",
                    ]
                    terminated = True
                elif op in {"call", "call_indirect"}:
                    if op == "call":
                        function_index = values[0]
                        type_index = self.function_types[function_index]
                        statements.append(f"_target = {function_index}")
                    else:
                        type_index = values[0]
                        statements.append(
                            f"_target = v.indirect({values[1]}, S.pop(), {type_index})"
                        )
                    nargs = len(self.types[type_index][0])
                    statements.append(f"_args = _take(S, {nargs})")
                    statements.append(f"return ('call', _target, _args, {next_block})")
                    terminated = True
                else:
                    statements = self.instruction(op, values)
                    terminated = op == "unreachable"
                output.extend("    " + statement for statement in statements)
                if terminated:
                    break
            if not terminated:
                output.append(f"    return {block + 1}")
            output.append("")
        output.append(
            "BLOCKS = ("
            + ", ".join(f"b{index}" for index in range(len(boundaries)))
            + ",)"
        )
        zeros = []
        for local in function.locals:
            zeros.extend(
                [
                    0.0
                    if local.type.data in {0x7C, 0x7D}
                    else None
                    if local.type.data in {0x70, 0x6F}
                    else 0
                ]
                * local.n
            )
        return "\n".join(output) + "\n", {
            "type": function.type,
            "zeros": zeros,
            "labels": len(labels),
            "blocks": len(boundaries),
        }

    def instruction(self, op, values):
        if op == "unreachable":
            return ["raise Trap('unreachable')"]
        if op == "nop":
            return ["pass"]
        if op == "drop":
            return ["S.pop()"]
        if op == "select":
            return [
                "_condition=S.pop(); _right=S.pop(); _left=S.pop()",
                "S.append(_left if _condition else _right)",
            ]
        if op in {"local.get", "global.get"}:
            return [
                f"S.append({'L' if op.startswith('local') else 'v.globals'}[{values[0]}])"
            ]
        if op in {"local.set", "global.set"}:
            return [
                f"{'L' if op.startswith('local') else 'v.globals'}[{values[0]}] = S.pop()"
            ]
        if op == "local.tee":
            return [f"L[{values[0]}] = S[-1]"]
        if op.endswith(".const"):
            if op.startswith("i"):
                bits = int(op[1:3])
                return [f"S.append({values[0] & ((1 << bits) - 1)})"]
            bits = int(op[1:3])
            raw = int.from_bytes(
                struct.pack("<f" if bits == 32 else "<d", values[0]), "little"
            )
            return [f"S.append(_float_bits({raw}, {bits}))"]
        if ".load" in op:
            kind, operation = op.split(".")
            bits = int(kind[1:])
            width = int("".join(filter(str.isdigit, operation)) or bits) // 8
            signed = operation.endswith("_s")
            return [
                f"S.append(v.load(S.pop()+{values[1]}, {width}, {signed}, {kind.startswith('f')}, {bits}))"
            ]
        if ".store" in op:
            kind, operation = op.split(".")
            bits = int(kind[1:])
            width = int("".join(filter(str.isdigit, operation)) or bits) // 8
            return [
                "_value=S.pop()",
                f"v.store(S.pop()+{values[1]}, _value, {width}, {kind.startswith('f')})",
            ]
        if op == "memory.copy":
            return [
                "_size=S.pop(); _source=S.pop(); _destination=S.pop()",
                "v.copy(_destination, _source, _size)",
            ]
        if op == "memory.fill":
            return [
                "_size=S.pop(); _value=S.pop(); _destination=S.pop()",
                "v.fill(_destination, _value, _size)",
            ]
        if op == "memory.size":
            return ["S.append(v.memory.size)"]
        if op == "memory.grow":
            return ["S.append(v.grow(S.pop()))"]
        if op == "i32.wrap_i64":
            return ["S.append(S.pop() & 4294967295)"]
        if op.startswith("i64.extend_i32"):
            return [
                "S.append("
                + (
                    "_signed(S.pop(),32) & 18446744073709551615"
                    if op.endswith("_s")
                    else "S.pop() & 4294967295"
                )
                + ")"
            ]
        if ".extend" in op and op.endswith("_s"):
            bits = int(op[1:3])
            source = int(op.split("extend")[1].split("_")[0])
            return [
                f"S.append(_signed(S.pop() & {(1 << source) - 1},{source}) & {(1 << bits) - 1})"
            ]
        if ".reinterpret_" in op:
            kind, source = op.split(".reinterpret_")
            bits = int(kind[1:])
            return [
                f"S.append({'_float_bits' if kind.startswith('f') else '_int_bits'}(S.pop(), {bits}))"
            ]
        if op == "f64.promote_f32":
            return ["S.append(float(S.pop()))"]
        if op.startswith("f64.convert_i64"):
            return [
                "S.append(float("
                + ("_signed(S.pop(),64)" if op.endswith("_s") else "S.pop()")
                + "))"
            ]
        if op.startswith(("i32.", "i64.")):
            bits = int(op[1:3])
            name = op.split(".")[1]
            mask = (1 << bits) - 1
            if name == "eqz":
                return ["S.append(int(S.pop()==0))"]
            if name == "clz":
                return [f"S.append({bits}-S.pop().bit_length())"]
            if name == "ctz":
                return [
                    f"_value=S.pop(); S.append((_value & -_value).bit_length()-1 if _value else {bits})"
                ]
            if name == "popcnt":
                return ["S.append(bin(S.pop()).count('1'))"]
            prefix = ["_right=S.pop(); _left=S.pop()"]
            comparison = {
                "eq": "==",
                "ne": "!=",
                "lt": "<",
                "le": "<=",
                "gt": ">",
                "ge": ">=",
            }
            stem = name.split("_")[0]
            if stem in comparison:
                left = f"_signed(_left,{bits})" if name.endswith("_s") else "_left"
                right = f"_signed(_right,{bits})" if name.endswith("_s") else "_right"
                return prefix + [f"S.append(int({left} {comparison[stem]} {right}))"]
            operations = {
                "add": "+",
                "sub": "-",
                "mul": "*",
                "and": "&",
                "or": "|",
                "xor": "^",
            }
            if name in operations:
                return prefix + [
                    f"S.append((_left {operations[name]} _right) & {mask})"
                ]
            if name in {"shl", "shr_s", "shr_u"}:
                left = f"_signed(_left,{bits})" if name == "shr_s" else "_left"
                operator = "<<" if name == "shl" else ">>"
                return prefix + [
                    f"S.append(({left} {operator} (_right & {bits - 1})) & {mask})"
                ]
            if name in {"rotl", "rotr"}:
                return prefix + [
                    f"S.append(_rotate(_left, _right, {bits}, {name == 'rotl'}))"
                ]
            if name in {"div_u", "div_s", "rem_u", "rem_s"}:
                return prefix + [
                    f"S.append(_integer_div(_left,_right,{bits},{name.endswith('_s')},{name.startswith('rem')}))"
                ]
        if op.startswith(("f32.", "f64.")):
            bits = int(op[1:3])
            name = op.split(".")[1]
            unary = {
                "abs": "abs(S.pop())",
                "neg": "-S.pop()",
                "trunc": "_trunc(S.pop())",
            }
            if name in unary:
                return [f"S.append({unary[name]})"]
            prefix = ["_right=S.pop(); _left=S.pop()"]
            comparison = {
                "eq": "==",
                "ne": "!=",
                "lt": "<",
                "le": "<=",
                "gt": ">",
                "ge": ">=",
            }
            if name in comparison:
                return prefix + [f"S.append(int(_left {comparison[name]} _right))"]
            operation = {"add": "+", "sub": "-", "mul": "*"}
            if name in operation:
                expression = f"_left {operation[name]} _right"
                return prefix + [
                    f"S.append(_round_float({expression}, 32))"
                    if bits == 32
                    else f"S.append({expression})"
                ]
            if name == "div":
                return prefix + [
                    "S.append(_round_float(_float_div(_left,_right), 32))"
                    if bits == 32
                    else "S.append(_float_div(_left,_right))"
                ]
        raise ValueError(f"Untranslated WebAssembly instruction: {op} {values}")


def constant(expression, opcode):
    if len(expression.data) != 1:
        raise ValueError("Nonconstant initializer")
    value = expression.data[0]
    name = opcode.name[value.opcode]
    if name in {"i32.const", "i64.const"}:
        return value.args[0] & ((1 << int(name[1:3])) - 1)
    if name == "ref.func":
        return value.args[0]
    raise ValueError(f"Unsupported initializer {name}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wasm", required=True, type=Path)
    parser.add_argument("--pywasm", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--entry")
    parser.add_argument("--initializer", action="append", default=[])
    args = parser.parse_args()
    sys.path.insert(0, str(args.pywasm))
    import pywasm

    @classmethod
    def limits(cls, stream):
        flags = stream.read(1)[0]
        if flags & ~5:
            raise ValueError("Shared or unsupported memory/table limits")
        minimum = pywasm.leb128.u.decode_reader(stream)[0]
        maximum = pywasm.leb128.u.decode_reader(stream)[0] if flags & 1 else 0
        result = cls(minimum, maximum)
        result.flags = flags
        return result

    pywasm.core.Limits.from_reader = limits

    with args.wasm.open("rb") as stream:
        module = pywasm.core.ModuleDesc.from_reader(stream)
        if stream.read():
            raise ValueError("WebAssembly parser left unread sections")
    if module.star != -1:
        raise ValueError("A module start section requires explicit lowering")
    if any(entry.kind != 0 for entry in module.imps) or len(module.mems) != 1:
        raise ValueError("Expected one memory and function-only imports")
    compiler = Generator(module, pywasm.opcode)
    args.output.mkdir(parents=True, exist_ok=True)
    code = bytearray()
    functions = []
    chunks = []
    chunk = bytearray()

    def flush_chunk():
        if not chunk:
            return
        payload = zlib.compress(chunk, 9)
        chunks.append(
            {"offset": len(code), "length": len(payload), "raw_length": len(chunk)}
        )
        code.extend(payload)
        chunk.clear()

    for index, function in enumerate(module.func):
        source, metadata = compiler.function(function)
        # Compile every block before accepting the generated module.
        compile(source, f"wasm-function-{index}", "exec")
        source = source.encode()
        if chunk and len(chunk) + len(source) > 1024 * 1024:
            flush_chunk()
        metadata.update(
            chunk=len(chunks),
            offset=len(chunk),
            length=len(source),
            source_sha256=hashlib.sha256(source).hexdigest(),
        )
        chunk.extend(source)
        functions.append(metadata)
    flush_chunk()
    data = bytearray()
    segments = []
    for entry in module.data:
        if entry.kind not in {0, 2}:
            raise ValueError("Passive data segment needs explicit lowering")
        segments.append(
            [constant(entry.offset, pywasm.opcode), len(data), len(entry.init)]
        )
        data.extend(entry.init)
    tables = []
    for table in module.tabl:
        tables.append([None] * table.limits.n)
    for entry in module.elem:
        if entry.kind not in {0, 2}:
            raise ValueError("Passive element segment needs explicit lowering")
        offset = constant(entry.offset, pywasm.opcode)
        values = [constant(value, pywasm.opcode) for value in entry.init]
        tables[entry.tidx][offset : offset + len(values)] = values
    compressed_data = zlib.compress(data, 9)
    manifest = {
        "wasm_sha256": hashlib.sha256(args.wasm.read_bytes()).hexdigest(),
        "types": compiler.types,
        "function_types": compiler.function_types,
        "imports": [[entry.module, entry.name, entry.desc] for entry in module.imps],
        "functions": functions,
        "globals": [constant(entry.init, pywasm.opcode) for entry in module.glob],
        "format": "python-basic-blocks-v2",
        "chunks": chunks,
        "tables": tables,
        "segments": segments,
        "memory": [module.mems[0].limits.n, module.mems[0].limits.m],
        "exports": {entry.name: [entry.kind, entry.desc] for entry in module.exps},
        "instructions": dict(compiler.counts),
        "code_sha256": hashlib.sha256(code).hexdigest(),
        "data_sha256": hashlib.sha256(compressed_data).hexdigest(),
    }
    if module.mems[0].limits.flags & 4:
        limits_ = module.mems[0].limits
        manifest["memory"] = [limits_.n, limits_.m if limits_.flags & 1 else None, 64]
    if args.entry:
        exports = manifest["exports"]
        entry = exports[args.entry]
        if entry[0] != 0:
            raise ValueError("Entrypoint must be a function")
        parameters, returns = compiler.types[compiler.function_types[entry[1]]]
        if returns not in [[], [127]]:
            raise ValueError("Entrypoint must return void or an i32 exit status")
        for initializer in args.initializer:
            init = exports[initializer]
            if init[0] != 0 or compiler.types[compiler.function_types[init[1]]] != (
                [],
                [],
            ):
                raise ValueError("Initializer must take no arguments and return void")
        manifest["entry"] = {
            "name": args.entry,
            "arguments": [0] * len(parameters),
            "return_status": bool(returns),
            "initializers": args.initializer,
        }
    (args.output / "_program.json").write_text(
        json.dumps(manifest, separators=(",", ":"))
    )
    (args.output / "_code.bin").write_bytes(code)
    (args.output / "_data.bin").write_bytes(compressed_data)
    print(
        json.dumps(
            {
                "functions": len(functions),
                "instructions": sum(compiler.counts.values()),
                "python_compressed_bytes": len(code),
                "data_bytes": len(data),
            }
        )
    )


if __name__ == "__main__":
    main()
