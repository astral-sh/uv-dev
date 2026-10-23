"""Small, explicit decoders for data received from GitHub and agents."""

import json
import math
from typing import Never, cast


def loads(value: str) -> object:
    """Decode JSON without silently discarding duplicate object fields."""
    return json.loads(
        value,
        object_pairs_hook=_unique_object,
        parse_constant=_reject_constant,
        parse_float=_finite_float,
    )


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON object field: {key!r}")
        result[key] = value
    return result


def _reject_constant(value: str) -> Never:
    raise ValueError(f"Invalid JSON numeric constant: {value}")


def _finite_float(value: str) -> float:
    result = float(value)
    if not math.isfinite(result):
        raise ValueError(f"JSON number is outside the finite range: {value}")
    return result


def as_object(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        raise TypeError("Expected a JSON object")
    if not all(isinstance(key, str) for key in value):
        raise ValueError("JSON object fields must be strings")
    return cast(dict[str, object], value)


def as_array(value: object) -> list[object]:
    if not isinstance(value, list):
        raise TypeError("Expected a JSON array")
    return cast(list[object], value)


def as_string(value: object) -> str:
    if not isinstance(value, str):
        raise TypeError("Expected a JSON string")
    return value


def as_positive_integer(value: object) -> int:
    if type(value) is not int or value <= 0:
        raise ValueError("Expected a positive JSON integer")
    return value
