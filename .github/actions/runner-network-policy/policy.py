"""Load the deliberately small, default-deny runner policy format."""

import ipaddress
import json
import re
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

HOSTNAME = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?")
PRIVATE_SERVICES = {
    977: "artifactcache.actions.githubusercontent.com",
    978: "results-receiver.actions.githubusercontent.com",
}


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

    def forbids(self, value):
        return any(matches(hostname(value), rule) for rule in self.deny)


def load(path, name):
    document = json.loads(Path(path).read_text())
    if document["version"] != 1:
        raise ValueError("unsupported policy version")
    return Policy.from_dict(document["profiles"][name])


def validate_private_origins(value):
    if not isinstance(value, dict) or len(value) > 2:
        raise ValueError("invalid private Actions services")
    for authority, target in value.items():
        parsed = urlsplit("http://" + authority)
        address = ipaddress.ip_address(parsed.hostname)
        if (
            address not in ipaddress.ip_network("10.0.0.0/8")
            or parsed.netloc != f"{address}:{parsed.port}"
            or parsed.path
            or parsed.query
            or parsed.fragment
            or PRIVATE_SERVICES.get(parsed.port) != target
        ):
            raise ValueError("unexpected private Actions service")
    return value


def private_origins(environment):
    result = {}
    for variable, port in (("ACTIONS_CACHE_URL", 977), ("ACTIONS_RESULTS_URL", 978)):
        if not (value := environment.get(variable)):
            continue
        parsed = urlsplit(value)
        if parsed.scheme == "https":
            if (
                not hostname(parsed.hostname).endswith(".actions.githubusercontent.com")
                or parsed.port not in {None, 443}
                or parsed.username
                or parsed.password
            ):
                raise ValueError("unexpected Actions service")
            continue
        if (
            parsed.scheme != "http"
            or parsed.port != port
            or parsed.username
            or parsed.password
            or parsed.fragment
        ):
            raise ValueError("unexpected private Actions service")
        result[parsed.netloc] = PRIVATE_SERVICES[port]
    return validate_private_origins(result)
