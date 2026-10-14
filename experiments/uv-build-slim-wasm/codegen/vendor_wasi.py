"""Vendor the pinned WASI host adapter without the WebAssembly interpreter."""

import argparse
import ast
from pathlib import Path


class Lower(ast.NodeTransformer):
    counter = 0

    def visit_Import(self, node):
        if len(node.names) == 1 and node.names[0].name == "os":
            return ast.ImportFrom(
                module="_posix", names=[ast.alias(name="os")], level=1
            )
        node.names = [name for name in node.names if name.name != "pywasm.core"]
        return node if node.names else None

    def visit_FunctionDef(self, node):
        if node.name in {"bind", "main"}:
            return None
        return self.generic_visit(node)

    def visit_Call(self, node):
        node = self.generic_visit(node)
        if (
            isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "os"
            and node.func.attr == "open"
            and len(node.args) >= 3
            and isinstance(node.args[2], ast.Constant)
            and node.args[2].value == 0o644
        ):
            node.args[2] = ast.Constant(value=0o666)
        if isinstance(node.func, ast.Attribute):
            receiver = node.func.value
            if (
                node.func.attr == "fileno"
                and isinstance(receiver, ast.Attribute)
                and isinstance(receiver.value, ast.Name)
                and receiver.value.id == "sys"
                and receiver.attr in {"stdin", "stdout", "stderr"}
            ):
                return ast.Constant(
                    value={"stdin": 0, "stdout": 1, "stderr": 2}[receiver.attr]
                )
            if (
                node.func.attr in {"encode", "decode"}
                and not node.args
                and not node.keywords
            ):
                node.keywords.append(
                    ast.keyword(
                        arg="errors", value=ast.Constant(value="surrogateescape")
                    )
                )
        return node

    def visit_Attribute(self, node):
        node = self.generic_visit(node)
        receiver = node.value
        if (
            node.attr == "name"
            and isinstance(receiver, ast.Attribute)
            and isinstance(receiver.value, ast.Name)
            and receiver.value.id == "sys"
            and receiver.attr in {"stdin", "stdout", "stderr"}
        ):
            return ast.Constant(value="<" + receiver.attr + ">")
        return node

    def visit_Match(self, node):
        self.counter += 1
        variable = "_matched_value_%d" % self.counter
        subject = ast.Name(id=variable, ctx=ast.Load())
        first = ast.Assign(
            targets=[ast.Name(id=variable, ctx=ast.Store())],
            value=self.visit(node.subject),
        )
        chain = []
        for case in reversed(node.cases):
            body = [self.visit(statement) for statement in case.body]
            body = [
                part
                for statement in body
                for part in (statement if isinstance(statement, list) else [statement])
                if part is not None
            ]
            if isinstance(case.pattern, ast.MatchValue):
                condition = ast.Compare(
                    left=subject, ops=[ast.Eq()], comparators=[case.pattern.value]
                )
            elif isinstance(case.pattern, ast.MatchAs):
                condition = ast.Constant(value=True)
                if case.pattern.name:
                    body.insert(
                        0,
                        ast.Assign(
                            targets=[ast.Name(id=case.pattern.name, ctx=ast.Store())],
                            value=subject,
                        ),
                    )
                    if case.guard:

                        class Rename(ast.NodeTransformer):
                            def visit_Name(self, name):
                                return subject if name.id == case.pattern.name else name

                        case.guard = Rename().visit(case.guard)
            else:
                raise ValueError("Unsupported WASI adapter pattern")
            if case.guard:
                condition = ast.BoolOp(op=ast.And(), values=[condition, case.guard])
            chain = [ast.If(test=condition, body=body, orelse=chain)]
        return [first, *chain]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    tree = Lower().visit(ast.parse(args.source.read_text()))
    tree.body.insert(
        0,
        ast.ImportFrom(
            module="__future__", names=[ast.alias(name="annotations")], level=0
        ),
    )
    ast.fix_missing_locations(tree)
    code = (
        "# Derived from libraries/pywasm at 47abba6c94ecf74c3c173344684891f2ca29c2e3.\n# MIT license; see LICENSE-PYWASM.\n"
        + ast.unparse(tree)
        + "\n"
    )
    ast.parse(code, feature_version=(3, 8))
    args.output.write_text(code)


if __name__ == "__main__":
    main()
