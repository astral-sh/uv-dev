"""Rust values, propagation, references, and iterators for AST-generated code."""

import bisect
import copy
import itertools
from functools import wraps

from . import _runtime as base
from ._runtime import (
    RustError,
    _String,
    _Some,
    _NONE,
    _display,
    _iter,
    _Struct as _Struct,
    _identity as _identity,
    _string_with_capacity as _string_with_capacity,
    _index as _index,
    _format as _format,
    _set_alphanumeric as _set_alphanumeric,
)


class _Error:
    def __init__(self, variant, template, *values, **fields):
        self.name, self.values, self.fields = variant, values, fields
        self.template = template

    def __str__(self):
        return self.template.format(
            *map(_display, self.values),
            **{key: _display(value) for key, value in self.fields.items()},
        )


class _Result:
    def __init__(self, ok, value):
        self.ok, self.value = ok, value

    def unwrap(self):
        if not self.ok:
            raise RustError(_display(self.value))
        return self.value

    def __str__(self):
        return _display(self.unwrap())


def _ok(value):
    return _Result(True, value)


def _err(value):
    return _Result(False, value)


class _Return(Exception):
    def __init__(self, value):
        self.value = value


def _try(value):
    if isinstance(value, _Result):
        if not value.ok:
            raise _Return(value)
        return value.value
    if value is _NONE:
        raise _Return(_NONE)
    if isinstance(value, _Some):
        return value.value
    raise TypeError("Rust ? requires a Result or Option")


def _rust_function(function):
    @wraps(function)
    def run(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except _Return as returned:
            return returned.value

    return run


class _Enum:
    def __init__(self, variant, *values, **fields):
        self.name, self.values, self.fields = variant, values, fields

    def __eq__(self, other):
        return isinstance(other, _Enum) and (self.name, self.values, self.fields) == (
            other.name,
            other.values,
            other.fields,
        )


class _Ref:
    def __init__(self, container, key):
        self.container, self.key = container, key

    @property
    def value(self):
        return self.container[self.key]

    @value.setter
    def value(self, value):
        self.container[self.key] = value


def _deref(value):
    return value.value if isinstance(value, _Ref) else value


def _set_ref(reference, value):
    reference.value = value


def _required():
    raise TypeError("A required Rust struct field was omitted")


def _repeat(value, count):
    return [copy.deepcopy(value) for _ in range(count)]


_ALPHABETIC = ()
_ALPHABETIC_STARTS = ()


def _set_alphabetic(ranges):
    global _ALPHABETIC, _ALPHABETIC_STARTS
    _ALPHABETIC = tuple(ranges)
    _ALPHABETIC_STARTS = tuple(first for first, _last in ranges)


def _write(writer, value, newline):
    base._write(writer, value, newline)
    return _ok(None)


def _method(value, name, *args, generic=None):
    generated = getattr(type(value), "_rust_methods", {}).get(name)
    if generated is not None:
        return generated(value, *args)
    if name in {"as_deref", "into", "as_slice", "as_ref", "as_str"}:
        return value
    if name in {"clone", "to_owned"}:
        return copy.deepcopy(value)
    if name == "map_err":
        if not isinstance(value, _Result):
            raise TypeError("map_err requires Result")
        return value if value.ok else _err(args[0](value.value))
    if name == "is_err":
        return isinstance(value, _Result) and not value.ok
    if name in {"expect", "unwrap"}:
        if isinstance(value, _Result):
            return value.unwrap()
        if isinstance(value, _Some):
            return value.value
        raise RuntimeError(args[0] if args else "called unwrap on None")
    if name == "ok_or":
        return _ok(value.value) if isinstance(value, _Some) else _err(args[0])
    if name == "is_some":
        return isinstance(value, _Some)
    if name == "unwrap_or_default":
        return value.value if isinstance(value, _Some) else []
    if name == "unwrap_or_else":
        return value.value if isinstance(value, _Some) else args[0]()
    if name == "map" and isinstance(value, _Result):
        return _ok(args[0](value.value)) if value.ok else value
    if name == "and_then":
        if isinstance(value, _Result):
            return args[0](value.value) if value.ok else value
        return args[0](value.value) if isinstance(value, _Some) else _NONE
    if name in {"into_iter", "by_ref"}:
        return _iter(value)
    if name == "iter_mut":
        return iter(_Ref(value, index) for index in range(len(value)))
    if name == "cloned":
        if value is _NONE:
            return _NONE
        if isinstance(value, _Some):
            return _Some(copy.deepcopy(value.value))
        return map(copy.deepcopy, _iter(value))
    if name == "enumerate":
        return enumerate(_iter(value))
    if name == "chain":
        return itertools.chain(_iter(value), _iter(args[0]))
    if name == "zip":
        return zip(_iter(value), _iter(args[0]))
    if name == "keys":
        return iter(value.keys())
    if name == "flat_map":
        return itertools.chain.from_iterable(
            _iter(args[0](item)) for item in _iter(value)
        )
    if name == "filter":
        if value is _NONE:
            return _NONE
        if isinstance(value, _Some):
            return value if args[0](value.value) else _NONE
        return filter(args[0], _iter(value))
    if name == "filter_map":
        return (
            mapped.value
            for item in _iter(value)
            if isinstance(mapped := args[0](item), _Some)
        )
    if name == "collect" and generic and "Result" in generic:
        result = []
        for item in _iter(value):
            if not item.ok:
                return item
            result.append(item.value)
        return _ok(result)
    if name == "is_alphabetic":
        code = ord(value)
        index = bisect.bisect_right(_ALPHABETIC_STARTS, code) - 1
        return index >= 0 and code <= _ALPHABETIC[index][1]
    if name == "find" and not isinstance(value, (str, _String)):
        for item in _iter(value):
            if args[0](item):
                return _Some(item)
        return _NONE
    if name == "split":
        return iter(_display(value).split(_display(args[0])))
    if name == "split_once":
        parts = _display(value).split(_display(args[0]), 1)
        return _Some(tuple(parts)) if len(parts) == 2 else _NONE
    if name == "trim":
        return _display(value).strip()
    if name == "trim_end":
        return _display(value).rstrip()
    if name == "contains":
        return args[0] in (_display(value) if isinstance(value, _String) else value)
    if name == "starts_with":
        return _display(value).startswith(_display(args[0]))
    if name == "join" and not hasattr(value, "joinpath"):
        return _display(args[0]).join(map(_display, _iter(value)))
    if name == "sort":
        value.sort(key=_display)
        return None
    adapter = getattr(value, "_rust_method", None)
    if adapter is not None:
        return adapter(name, *args)
    return base._method(value, name, *args)


def _match(value, pattern):
    kind = pattern["kind"]
    if kind == "path" and pattern["name"] != "None":
        return (
            {}
            if isinstance(value, _Enum)
            and value.name == pattern["name"]
            and not value.values
            and not value.fields
            else None
        )
    if kind == "constructor":
        name = pattern["name"]
        if name == "Some":
            return (
                _match(value.value, pattern["items"][0])
                if isinstance(value, _Some)
                else None
            )
        if name in {"Ok", "Err"}:
            return (
                _match(value.value, pattern["items"][0])
                if isinstance(value, _Result) and value.ok == (name == "Ok")
                else None
            )
        if not isinstance(value, (_Enum, _Error)) or value.name != name:
            return None
        return _match(value.values, {"kind": "tuple", "items": pattern["items"]})
    if kind == "struct":
        if not isinstance(value, (_Enum, _Error)) or value.name != pattern["name"]:
            return None
        result = {}
        for name, subpattern in pattern["fields"].items():
            if name not in value.fields:
                return None
            matched = _match(value.fields[name], subpattern)
            if matched is None:
                return None
            result.update(matched)
        return result
    if kind == "slice":
        return _match(tuple(value), {"kind": "tuple", "items": pattern["items"]})
    if kind == "tuple":
        if not isinstance(value, tuple) or len(value) != len(pattern["items"]):
            return None
        result = {}
        for item, subpattern in zip(value, pattern["items"]):
            matched = _match(item, subpattern)
            if matched is None:
                return None
            result.update(matched)
        return result
    if kind == "or":
        for item in pattern["items"]:
            matched = _match(value, item)
            if matched is not None:
                return matched
        return None
    return base._match(value, pattern)


def _external(name):
    from ._adapters import external

    return external(name)
