"""Small, explicit decoders for data received from GitHub and agents."""

import json
from typing import cast


def loads(value: str) -> object:
    return json.loads(value)


def as_object(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise ValueError("Expected a JSON object")
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
