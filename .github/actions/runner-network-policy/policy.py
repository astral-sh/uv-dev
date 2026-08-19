"""Load the deliberately small, default-deny runner policy format."""

import ipaddress
import json
import re
from dataclasses import dataclass
from pathlib import Path

HOSTNAME = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?")


def hostname(value):
    if not isinstance(value, str) or not value or len(value) > 254:
        raise ValueError("invalid hostname")
    value = value.removesuffix(".").lower()
    if not all(HOSTNAME.fullmatch(label) for label in value.split(".")):
        raise ValueError("invalid hostname")
    try:
        ipaddress.ip_address(value)
    except ValueError:
        return value
    raise ValueError("IP literals are not domain names")


def pattern(value):
    if value.startswith("*."):
        return "*." + hostname(value[2:])
    return hostname(value)


def matches(value, rule):
    return (
        value.endswith(rule[1:]) and value != rule[2:]
        if rule.startswith("*.")
        else value == rule
    )


@dataclass(frozen=True)
class Policy:
    allow: tuple[str, ...]
    deny: tuple[str, ...] = ()

    @classmethod
    def from_dict(cls, value):
        if set(value) != {"allow", "deny"}:
            raise ValueError("unexpected policy fields")
        return cls(
            tuple(pattern(item) for item in value["allow"]),
            tuple(pattern(item) for item in value["deny"]),
        )

    def permits(self, value):
        try:
            value = hostname(value)
        except ValueError:
            return False
        return not any(matches(value, rule) for rule in self.deny) and any(
            matches(value, rule) for rule in self.allow
        )


def load(path, name):
    document = json.loads(Path(path).read_text())
    if document["version"] != 1:
        raise ValueError("unsupported policy version")
    return Policy.from_dict(document["profiles"][name])
