"""Explicit Rust standard-library adapters used by generated functions.

This module is handwritten and part of the trusted implementation boundary.
"""

import bisect
import copy
import itertools
from typing import ClassVar


class RustError(ValueError):
    """An error returned by a translated Rust function."""


class _String:
    def __init__(self, value=""):
        self.value = _display(value)

    def __str__(self):
        return self.value

    def __eq__(self, other):
        return self.value == _display(other)


class _Struct:
    _fields: ClassVar[dict] = {}

    def __init__(self, *args, **kwargs):
        if len(args) > len(self._fields):
            raise TypeError("Too many struct fields")
        supplied = dict(zip(self._fields, args))
        if supplied.keys() & kwargs.keys():
            raise TypeError("Duplicate struct field")
        supplied.update(kwargs)
        if supplied.keys() - self._fields.keys():
            raise TypeError(
                f"Unknown struct fields: {supplied.keys() - self._fields.keys()}"
            )
        for name, factory in self._fields.items():
            setattr(self, name, supplied[name] if name in supplied else factory())


class _Some:
    def __init__(self, value):
        self.value = value


class _NoneOption:
    def __deepcopy__(self, memo):
        return self


_NONE = _NoneOption()


class _Error:
    def __init__(self, name, template, *values):
        self.name = name
        self.message = template.format(*map(_display, values))

    def __str__(self):
        return self.message


def _identity(value):
    return value


def _display(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _string_with_capacity(_capacity):
    return _String()


def _err(value):
    raise RustError(_display(value))


def _iter(value):
    if value is _NONE:
        return iter(())
    if isinstance(value, _Some):
        return iter((value.value,))
    if isinstance(value, dict):
        return iter(value.items())
    return iter(value)


def _rust_lines(value):
    # Rust str::lines recognizes LF and CRLF, without Python's extra separators.
    if not value:
        return iter(())
    parts = value.split("\n")
    terminated = len(parts) - 1
    for index in range(terminated):
        parts[index] = parts[index].removesuffix("\r")
    if value.endswith("\n"):
        parts.pop()
    return iter(parts)


_ALPHANUMERIC = ()
_ALPHANUMERIC_STARTS = ()


def _set_alphanumeric(ranges):
    global _ALPHANUMERIC, _ALPHANUMERIC_STARTS
    _ALPHANUMERIC = tuple(ranges)
    _ALPHANUMERIC_STARTS = tuple(first for first, _last in ranges)


def _method(value, name, *args):
    if name in {"as_ref", "as_str"}:
        return value
    if name == "clone":
        return copy.deepcopy(value)
    if name == "to_string":
        return _String(value)
    if name == "len":
        return (
            len(_display(value).encode("utf-8"))
            if isinstance(value, (str, _String))
            else len(value)
        )
    if name == "is_empty":
        return _method(value, "len") == 0
    if name == "is_none":
        return value is _NONE
    if name == "is_some_and":
        if value is _NONE:
            return False
        if isinstance(value, _Some):
            return args[0](value.value)
        raise TypeError("is_some_and requires Option")
    if name == "map":
        if value is _NONE:
            return _NONE
        if isinstance(value, _Some):
            return _Some(args[0](value.value))
        return map(args[0], _iter(value))
    if name == "iter":
        return _iter(value)
    if name == "flatten":
        return itertools.chain.from_iterable(_iter(item) for item in _iter(value))
    if name == "collect":
        return list(_iter(value))
    if name == "next":
        try:
            return _Some(next(value))
        except StopIteration:
            return _NONE
    if name == "all":
        return all(args[0](item) for item in _iter(value))
    if name == "bytes":
        return iter(_display(value).encode("utf-8"))
    if name == "chars":
        return iter(_display(value))
    if name == "lines":
        return _rust_lines(_display(value))
    if name == "to_ascii_lowercase":
        if not isinstance(value, int) or not 0 <= value <= 255:
            raise TypeError("This adapter implements u8::to_ascii_lowercase")
        return value + 32 if 65 <= value <= 90 else value
    if name == "is_alphanumeric":
        code = ord(value)
        index = bisect.bisect_right(_ALPHANUMERIC_STARTS, code) - 1
        return index >= 0 and code <= _ALPHANUMERIC[index][1]
    if name == "find":
        position = (
            _display(value).encode("utf-8").find(_display(args[0]).encode("utf-8"))
        )
        return _NONE if position < 0 else _Some(position)
    if name == "push":
        if isinstance(value, _String):
            value.value += _display(args[0])
        else:
            value.append(args[0])
        return None
    if name == "push_str":
        value.value += _display(args[0])
        return None
    if name == "extend":
        if isinstance(value, _String):
            value.value += "".join(map(_display, _iter(args[0])))
        else:
            value.extend(_iter(args[0]))
        return None
    if name == "repeat":
        return _display(value) * args[0]
    if name in {"python_tags", "abi_tags", "platform_tags"}:
        return getattr(value, name)
    raise NotImplementedError(f"Unmapped Rust method: {name}")


def _index(value, index):
    if isinstance(value, (str, _String)):
        sliced = _display(value).encode("utf-8")[index]
        if not isinstance(sliced, bytes):
            raise TypeError("Rust str cannot be indexed by a single integer")
        return sliced.decode("utf-8")
    return value[index]


def _format(template, args, captured):
    return template.format(
        *map(_display, args),
        **{key: _display(value) for key, value in captured.items()},
    )


def _write(writer, value, newline):
    writer.value += value + ("\n" if newline else "")


def _match(value, pattern):
    match pattern["kind"]:
        case "wild":
            return {}
        case "bind":
            return {pattern["name"]: value}
        case "literal":
            return {} if value == pattern["value"] else None
        case "path":
            if pattern["name"] != "None":
                raise NotImplementedError(pattern)
            return {} if value is _NONE else None
        case "range":
            start, end = pattern["start"], pattern["end"]
            if any(bound and bound["kind"] != "literal" for bound in (start, end)):
                raise NotImplementedError("Nonliteral pattern bound")
            if start and value < start["value"]:
                return None
            if end and (
                value > end["value"] if pattern["inclusive"] else value >= end["value"]
            ):
                return None
            return {}
        case "or":
            for item in pattern["items"]:
                result = _match(value, item)
                if result is not None:
                    return result
            return None
        case "constructor":
            if pattern["name"] != "Some" or len(pattern["items"]) != 1:
                raise NotImplementedError(pattern)
            return (
                _match(value.value, pattern["items"][0])
                if isinstance(value, _Some)
                else None
            )
        case "tuple":
            if not isinstance(value, tuple) or len(value) != len(pattern["items"]):
                return None
            result = {}
            for item, subpattern in zip(value, pattern["items"]):
                matched = _match(item, subpattern)
                if matched is None:
                    return None
                result.update(matched)
            return result
        case _:
            raise NotImplementedError(pattern)
