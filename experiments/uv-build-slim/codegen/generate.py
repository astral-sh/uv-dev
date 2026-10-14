"""Lower the closed Rust AST to ordinary Python functions.

Run the Rust frontend first, then pass its JSON output with --ast. The generated
module ships with the backend; neither Rust nor this generator runs at install time.
"""

import argparse
import ast
import hashlib
import json
import re
import symtable
from pathlib import Path


class UnsupportedRust(ValueError):
    pass


METHODS = {
    "to_string",
    "as_ref",
    "as_str",
    "is_empty",
    "len",
    "bytes",
    "chars",
    "push",
    "push_str",
    "extend",
    "lines",
    "next",
    "repeat",
    "iter",
    "map",
    "flatten",
    "collect",
    "all",
    "is_none",
    "is_some_and",
    "to_ascii_lowercase",
    "is_alphanumeric",
    "find",
    "python_tags",
    "abi_tags",
    "platform_tags",
    "clone",
}
METHODS |= {
    "and",
    "and_then",
    "as_deref",
    "chain",
    "cloned",
    "compile_matcher",
    "contains",
    "enumerate",
    "expect",
    "extension",
    "file_name",
    "file_type",
    "filter",
    "filter_entry",
    "filter_map",
    "flat_map",
    "into",
    "into_iter",
    "is_alphabetic",
    "is_err",
    "is_file",
    "is_match",
    "is_some",
    "is_symlink",
    "iter_mut",
    "join",
    "keys",
    "license_metadata",
    "map_err",
    "match_directory",
    "match_path",
    "ok_or",
    "parse",
    "path",
    "portable_display",
    "sort",
    "sort_by_file_name",
    "split",
    "split_once",
    "starts_with",
    "strip_prefix",
    "to_owned",
    "to_path_buf",
    "trim",
    "trim_end",
    "unwrap_or_default",
    "user_display",
    "zip",
    "by_ref",
    "peekable",
    "peek",
}

EXTERNALS = {
    "GlobDirFilter::from_globs",
    "MarkerTree::expression",
    "MarkerValueExtra::Extra",
    "WalkDir::new",
    "error_on_venv",
    "fs_err::read",
    "fs_err::read_to_string",
    "spdx::Expression::parse",
    "str::from_utf8",
    "OsStr::to_str",
    "ToString::to_string",
    "Into::into",
    "PortableGlobParser::Pep639",
    "ExtraOperator::Equal",
}


def symbol(name):
    return "_rs_" + name.replace("::", "__")


def names(pattern):
    if pattern["kind"] == "bind":
        return [pattern["name"]]
    result = []
    for item in pattern.get("items", []):
        result.extend(names(item))
    for item in pattern.get("fields", {}).values():
        result.extend(names(item))
    return list(dict.fromkeys(result))


class Emitter:
    def __init__(self, tree):
        self.tree = tree
        self.lines = []
        self.indent = 0
        self.counter = 0
        self.owner = None

    def line(self, text):
        self.lines.append("    " * self.indent + text)

    def temp(self):
        self.counter += 1
        return f"_temporary_{self.counter}"

    def target(self, pattern):
        match pattern["kind"]:
            case "bind":
                return pattern["name"]
            case "wild":
                return "_"
            case "tuple":
                return "(" + ", ".join(self.target(p) for p in pattern["items"]) + ",)"
            case _:
                raise UnsupportedRust(f"Unsupported assignment pattern: {pattern}")

    def bindings(self, pattern, matched):
        for name in names(pattern):
            self.line(f"{name} = {matched}[{name!r}]")

    def resolve(self, name):
        if name.startswith("Self::"):
            name = self.owner + name[4:]
        if name in self.tree["functions"]:
            return symbol(name)
        if name in self.tree["errors"]:
            return f"_Error({name!r}, _ERROR_MESSAGES[{name!r}])"
        if name in self.tree.get("variants", {}):
            return f"_Enum({name!r})"
        if name in EXTERNALS:
            return f"_external({name!r})"
        if name == "Self":
            return f"_{self.owner}"
        builtins = {
            "String::new": "_String",
            "String::with_capacity": "_string_with_capacity",
            "Vec::new": "list",
            "SmallString::from": "_display",
            "PathBuf::from": "_display",
            "Cow::Owned": "_identity",
            "Cow::Borrowed": "_identity",
            "Some": "_Some",
            "None": "_NONE",
            "Ok": "_ok",
            "Err": "_err",
            "WheelDataIncludes::default": "_WheelDataIncludes",
            "InvalidNameError": "_InvalidNameError",
            "Keywords::new": "_Keywords",
            "ProjectUrls::new": "_ProjectUrls",
            "Vec::with_capacity": "lambda _capacity: []",
        }
        if name in builtins:
            return builtins[name]
        if "::" in name:
            raise UnsupportedRust(f"Unmapped Rust symbol: {name}")
        return name

    def block(self, statements, result=None, returning=False):
        if not statements:
            self.line("pass")
        for index, statement in enumerate(statements):
            match statement["kind"]:
                case "local":
                    if statement.get("else") is not None:
                        expression = self.expression(statement["value"])
                        matched = self.temp()
                        self.line(
                            f"{matched} = _match({expression}, {statement['pattern']!r})"
                        )
                        self.line(f"if {matched} is None:")
                        self.indent += 1
                        self.value_statement(statement["else"])
                        self.indent -= 1
                        self.bindings(statement["pattern"], matched)
                    else:
                        target = self.target(statement["pattern"])
                        self.value_statement(statement["value"], target)
                case "function":
                    self.function(statement["value"], statement["value"]["name"])
                case "expression":
                    tail = statement["tail"] and index == len(statements) - 1
                    self.value_statement(
                        statement["value"], result if tail else None, returning and tail
                    )
                case _:
                    raise UnsupportedRust(f"Unmapped statement: {statement}")

    def function(self, function, name):
        params = ", ".join(self.target(p) for p in function["params"])
        self.line("@_rust_function")
        self.line(f"def {name}({params}):")
        self.indent += 1
        self.block(function["body"], returning=True)
        self.indent -= 1
        self.line("")

    def value_statement(self, value, target=None, returning=False):
        match value["kind"]:
            case "if":
                condition = value["condition"]
                if condition["kind"] == "let":
                    expression = self.expression(condition["value"])
                    matched = self.temp()
                    self.line(
                        f"{matched} = _match({expression}, {condition['pattern']!r})"
                    )
                    self.line(f"if {matched} is not None:")
                    self.indent += 1
                    self.bindings(condition["pattern"], matched)
                else:
                    expression = self.expression(condition)
                    self.line(f"if {expression}:")
                    self.indent += 1
                self.block(value["then"], target, returning)
                self.indent -= 1
                if value["else"] is not None:
                    self.line("else:")
                    self.indent += 1
                    self.value_statement(value["else"], target, returning)
                    self.indent -= 1
                elif target is not None:
                    self.line("else:")
                    self.indent += 1
                    self.line(f"{target} = None")
                    self.indent -= 1
            case "match":
                subject = self.temp()
                self.line(f"{subject} = {self.expression(value['value'])}")
                done = self.temp()
                self.line(f"{done} = False")
                for arm in value["arms"]:
                    if arm.get("guard") is not None:
                        raise UnsupportedRust(
                            "Match guards require an additional lowering"
                        )
                    matched = self.temp()
                    self.line(f"if not {done}:")
                    self.indent += 1
                    self.line(f"{matched} = _match({subject}, {arm['pattern']!r})")
                    self.line(f"if {matched} is not None:")
                    self.indent += 1
                    self.line(f"{done} = True")
                    self.bindings(arm["pattern"], matched)
                    self.value_statement(arm["body"], target, returning)
                    self.indent -= 2
                self.line(f"if not {done}:")
                self.indent += 1
                self.line("raise RuntimeError('Non-exhaustive generated Rust match')")
                self.indent -= 1
            case "block":
                self.block(value["body"], target, returning)
            case "for":
                expression = self.expression(value["value"])
                self.line(
                    f"for {self.target(value['pattern'])} in _iter({expression}):"
                )
                self.indent += 1
                self.block(value["body"])
                self.indent -= 1
            case "continue" | "break":
                self.line(value["kind"])
            case "return":
                expression = (
                    self.expression(value["value"]) if value["value"] else "None"
                )
                self.line(f"return {expression}")
            case "assign":
                if (
                    value["left"]["kind"] == "unary"
                    and value["left"]["operator"] == "*"
                ):
                    self.line(
                        f"_set_ref({self.expression(value['left']['value'])}, {self.expression(value['right'])})"
                    )
                    return
                self.line(
                    f"{self.expression(value['left'])} = {self.expression(value['right'])}"
                )
            case _:
                expression = self.expression(value)
                prefix = "return " if returning else f"{target} = " if target else ""
                self.line(prefix + expression)

    def expression(self, value):
        kind = value["kind"]
        match kind:
            case "literal":
                return repr(value["value"])
            case "path":
                return self.resolve(value["name"])
            case "field":
                field = value["field"]
                return f"{self.expression(value['value'])}.{'_' if field.isdigit() else ''}{field}"
            case "try":
                return f"_try({self.expression(value['value'])})"
            case "call":
                function = value["function"]
                args = ", ".join(self.expression(a) for a in value["args"])
                if (
                    function["kind"] == "path"
                    and function["name"] in self.tree["errors"]
                ):
                    name = function["name"]
                    return f"_Error({name!r}, _ERROR_MESSAGES[{name!r}], {args})"
                if function["kind"] == "path" and function["name"] in self.tree.get(
                    "variants", {}
                ):
                    return f"_Enum({function['name']!r}, {args})"
                return f"({self.expression(function)})({args})"
            case "method":
                receiver = self.expression(value["value"])
                args = [self.expression(a) for a in value["args"]]
                method = value["name"]
                generated_methods = {
                    "to_vec_str": "ProjectUrls::to_vec_str",
                    "as_metadata": "Keywords::as_metadata",
                }
                if method in generated_methods:
                    return f"{symbol(generated_methods[method])}({', '.join([receiver, *args])})"
                if method not in METHODS:
                    raise UnsupportedRust(f"Unmapped Rust method: {method}")
                generic = (
                    f", generic={value['generic']!r}" if value.get("generic") else ""
                )
                return f"_method({', '.join([receiver, repr(method), *args])}{generic})"
            case "binary":
                operator = {
                    "&&": "and",
                    "||": "or",
                    "+": "+",
                    "==": "==",
                    "!=": "!=",
                }.get(value["operator"])
                if operator is None:
                    raise UnsupportedRust(f"Unsupported operator: {value['operator']}")
                left = self.expression(value["left"])
                if operator in {"and", "or"}:
                    result = self.temp()
                    self.line(f"{result} = {left}")
                    self.line(f"if {'not ' if operator == 'or' else ''}{result}:")
                    self.indent += 1
                    right = self.expression(value["right"])
                    self.line(f"{result} = {right}")
                    self.indent -= 1
                    return result
                right = self.expression(value["right"])
                return f"({left} {operator} {right})"
            case "unary":
                if value["operator"] == "*":
                    return f"_deref({self.expression(value['value'])})"
                if value["operator"] != "!":
                    raise UnsupportedRust(
                        f"Unsupported unary operator: {value['operator']}"
                    )
                return f"(not {self.expression(value['value'])})"
            case "tuple" | "array":
                items = ", ".join(self.expression(e) for e in value["items"])
                if kind == "array":
                    return f"[{items}]"
                return f"({items},)" if items else "None"
            case "struct":
                name = self.owner if value["name"] == "Self" else value["name"]
                args = ", ".join(
                    f"{field}={self.expression(expression)}"
                    for field, expression in value["fields"].items()
                )
                if name in self.tree["errors"]:
                    return f"_Error({name!r}, _ERROR_MESSAGES[{name!r}], {args})"
                if name in self.tree.get("variants", {}):
                    return f"_Enum({name!r}, {args})"
                if name == "MarkerExpression::Extra":
                    return f"_Enum({name!r}, {args})"
                if name not in self.tree["structs"]:
                    raise UnsupportedRust(f"Unknown struct: {name}")
                args = ", ".join(
                    f"{name}={self.expression(e)}"
                    for name, e in value["fields"].items()
                )
                return f"_{name}({args})"
            case "index":
                return f"_index({self.expression(value['value'])}, {self.expression(value['index'])})"
            case "range":
                start = self.expression(value["start"]) if value["start"] else "None"
                end = self.expression(value["end"]) if value["end"] else "None"
                if value["inclusive"]:
                    end = f"({end} + 1)"
                return f"slice({start}, {end})"
            case "cast":
                if value["type"] != "char":
                    raise UnsupportedRust(f"Unsupported cast: {value['type']}")
                return f"chr({self.expression(value['value'])})"
            case "closure":
                function = self.temp()
                arguments = [self.temp() for _ in value["params"]]
                self.line("@_rust_function")
                self.line(f"def {function}({', '.join(arguments)}):")
                self.indent += 1
                for pattern, argument in zip(value["params"], arguments):
                    self.line(f"{self.target(pattern)} = {argument}")
                self.value_statement(value["body"], returning=True)
                self.indent -= 1
                return function
            case "matches":
                return f"(_match({self.expression(value['value'])}, {value['pattern']!r}) is not None)"
            case "macro":
                name, args = value["name"], value["args"]
                if name == "vec":
                    return f"[{', '.join(self.expression(a) for a in args)}]"
                if name == "warn_user_once":
                    return "_external('warn_user_once')()"
                if name in {"warn", "debug", "trace"}:
                    return "None  # tracing is disabled in the PoC runtime"
                destination = None
                if name in {"write", "writeln"}:
                    destination = self.expression(args[0])
                    args = args[1:]
                if args[0]["kind"] != "literal" or not isinstance(
                    args[0]["value"], str
                ):
                    raise UnsupportedRust("Formatting requires a literal format string")
                template = args[0]["value"]
                captures = sorted(
                    set(re.findall(r"(?<!\{)\{([A-Za-z_][A-Za-z_0-9]*)\}", template))
                )
                captured = (
                    "{" + ", ".join(f"{name!r}: {name}" for name in captures) + "}"
                )
                positional = "[" + ", ".join(self.expression(a) for a in args[1:]) + "]"
                rendered = f"_format({template!r}, {positional}, {captured})"
                if destination:
                    return f"_write({destination}, {rendered}, {name == 'writeln'})"
                return rendered
            case "if" | "match" | "block":
                result = self.temp()
                self.value_statement(value, result)
                return result
            case "repeat":
                return f"_repeat({self.expression(value['value'])}, {self.expression(value['count'])})"
            case _:
                raise UnsupportedRust(f"Unsupported expression: {value}")

    def module(self, manifest):
        self.line('"""Generated from pinned uv Rust sources. DO NOT EDIT."""')
        self.line("from ._rust_runtime import (")
        self.line("    _String, _Struct, _Some, _NONE, _Error, _display, _identity,")
        self.line("    _string_with_capacity, _err, _iter, _method, _match, _index,")
        self.line("    _format, _write, _set_alphanumeric,")
        self.line("    _ok, _try, _rust_function, _Enum, _deref, _set_ref, _required,")
        self.line("    _repeat, _external, _set_alphabetic,")
        self.line(")")
        self.line(f"UPSTREAM_REVISION = {manifest['revision']!r}")
        self.line(f"UPSTREAM_VERSION = {manifest['version']!r}")
        self.line(
            f"SOURCE_SHA256 = { {f['path']: f['sha256'] for f in manifest['files']}!r}"
        )
        self.line(f"_ERROR_MESSAGES = {self.tree['errors']!r}")
        self.line(f"_set_alphanumeric({self.tree['alphanumeric']!r})")
        self.line(f"_set_alphabetic({self.tree.get('alphabetic', [])!r})")
        self.line("")
        for name, fields in self.tree["structs"].items():
            defaults = {}
            for field in fields:
                ty = field["type"]
                if ty.startswith("Option <"):
                    factory = "lambda: _NONE"
                elif ty.startswith("Vec <"):
                    factory = "list"
                elif ty == "String" or ty == "PathBuf":
                    factory = "_String"
                elif ty.startswith("IndexMap <"):
                    factory = "dict"
                elif ty == "bool":
                    factory = "bool"
                elif ty in self.tree["structs"]:
                    factory = f"lambda: _{ty}()"
                else:
                    factory = "_required"
                key = "_" + field["name"] if field["name"].isdigit() else field["name"]
                defaults[key] = factory
            self.line(f"class _{name}(_Struct):")
            self.line(
                "    _fields = {"
                + ", ".join(f"{k!r}: {v}" for k, v in defaults.items())
                + "}"
            )
            self.line("")
        for name, function in self.tree["functions"].items():
            self.owner = function.get("owner")
            self.line(f"# Rust: {name}")
            self.function(function, symbol(name))
        owners = {function.get("owner") for function in self.tree["functions"].values()}
        for owner in sorted(name for name in owners if name in self.tree["structs"]):
            methods = {
                name.split("::")[-1]: symbol(name)
                for name, function in self.tree["functions"].items()
                if function.get("owner") == owner
            }
            self.line(
                f"_{owner}._rust_methods = {{"
                + ", ".join(
                    f"{name!r}: {function}" for name, function in methods.items()
                )
                + "}"
            )
        for name in sorted(
            owner
            for owner in owners
            if f"{owner}::fmt" in self.tree["functions"]
            and owner in self.tree["structs"]
        ):
            self.line(f"def _display_{name}(self):")
            self.line("    writer = _String()")
            self.line(f"    {symbol(name + '::fmt')}(self, writer)")
            self.line("    return str(writer)")
            self.line(f"_{name}.__str__ = _display_{name}")
        return "\n".join(self.lines) + "\n"


def generate(tree, manifest):
    code = Emitter(tree).module(manifest)
    ast.parse(code)
    symbols = symtable.symtable(code, "_generated.py", "exec")
    available = {
        symbol.get_name()
        for symbol in symbols.get_symbols()
        if symbol.is_assigned() or symbol.is_imported() or symbol.is_namespace()
    } | {"str", "list", "dict", "bool", "chr", "slice", "RuntimeError"}

    def check_scope(scope):
        for symbol in scope.get_symbols():
            if (
                symbol.is_referenced()
                and symbol.is_global()
                and symbol.get_name() not in available
            ):
                raise UnsupportedRust(f"Untranslated Rust symbol: {symbol.get_name()}")
        for child in scope.get_children():
            check_scope(child)

    check_scope(symbols)
    return code


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ast", type=Path, required=True)
    parser.add_argument(
        "--vendor", type=Path, default=Path(__file__).resolve().parents[1] / "vendor"
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "src/uv_build_slim/_generated.py",
    )
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    manifest = json.loads((args.vendor / "manifest.json").read_text())
    for source in manifest["files"]:
        actual = hashlib.sha256((args.vendor / source["path"]).read_bytes()).hexdigest()
        if actual != source["sha256"]:
            raise SystemExit(f"Source hash mismatch: {source['path']}")
    tree = json.loads(args.ast.read_text())
    code = generate(tree, manifest)
    if args.check:
        if args.output.read_text() != code:
            raise SystemExit("Generated Python is stale")
    else:
        args.output.write_text(code)
    print(
        f"{'Checked' if args.check else 'Generated'} {len(tree['functions'])} Rust functions"
    )


if __name__ == "__main__":
    main()
