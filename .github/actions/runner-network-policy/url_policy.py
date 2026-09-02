"""Match reviewed HTTP URLs without normalizing request paths or queries."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

from policy import hostname

DEFAULT_PORTS = {"http": 80, "https": 443}
METHODS = frozenset({"GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"})
HEX_DIGITS = frozenset("0123456789abcdefABCDEF")
UNRESERVED = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~"
SUB_DELIMITERS = "!$&'()*+,;="
# Some origins remove semicolon path parameters before resolving dot segments.
PATH_CHARACTERS = frozenset(UNRESERVED + SUB_DELIMITERS + ":@/") - {";"}
QUERY_CHARACTERS = PATH_CHARACTERS | frozenset("?;")
ENCODED_PATH_DELIMITERS = frozenset(":/?#[]@!$&'()*+,;=.%\\")
IPV4_SPELLING = re.compile(r"(?:0x[0-9a-f]+|[0-9]+)(?:\.(?:0x[0-9a-f]+|[0-9]+)){0,3}")
INTEGER_SEGMENT = "{integer}"


def visible_ascii(value: str) -> None:
    if not isinstance(value, str) or any(
        not 0x21 <= ord(character) <= 0x7E for character in value
    ):
        raise ValueError("URL must use visible ASCII")


def canonical_hostname(value: str) -> str:
    visible_ascii(value)
    result = hostname(value)
    if IPV4_SPELLING.fullmatch(result):
        raise ValueError("IP literals are not domain names")
    return result


def validate_component(value: str, *, path: bool) -> None:
    allowed = PATH_CHARACTERS if path else QUERY_CHARACTERS
    index = 0
    while index < len(value):
        character = value[index]
        if character != "%":
            if character not in allowed:
                raise ValueError("invalid URL character")
            index += 1
            continue
        encoded = value[index + 1 : index + 3]
        if len(encoded) != 2 or any(item not in HEX_DIGITS for item in encoded):
            raise ValueError("invalid URL escape")
        decoded = int(encoded, 16)
        if (
            decoded < 0x20
            or decoded >= 0x7F
            or (path and (decoded == 0x20 or chr(decoded) in ENCODED_PATH_DELIMITERS))
        ):
            raise ValueError("ambiguous URL escape")
        index += 3


def request_target(value: str) -> tuple[str, str]:
    """Return the unchanged path and query suffix, retaining a bare question mark."""
    visible_ascii(value)
    path, separator, query = value.partition("?")
    if (
        not path.startswith("/")
        or "//" in path
        or any(segment in {".", ".."} for segment in path.split("/"))
    ):
        raise ValueError("ambiguous request path")
    validate_component(path, path=True)
    validate_component(query, path=False)
    return path, separator + query


def parse_url(value: str, *, template: bool = False) -> tuple[str, str, int, str, str]:
    """Parse the small HTTP URL grammar without permissive URL-parser rewriting."""
    visible_ascii(value)
    scheme, separator, remainder = value.partition("://")
    scheme = scheme.lower()
    if not separator or scheme not in DEFAULT_PORTS:
        raise ValueError("HTTP or HTTPS URL required")
    authority_end = min(
        (index for item in "/?" if (index := remainder.find(item)) >= 0),
        default=len(remainder),
    )
    authority = remainder[:authority_end]
    host, separator, port = authority.partition(":")
    host = canonical_hostname(host)
    if separator and port != str(DEFAULT_PORTS[scheme]):
        raise ValueError("only the scheme's default port is allowed")
    target = remainder[authority_end:]
    if not target or target.startswith("?"):
        target = "/" + target
    if template:
        path, separator, query = target.partition("?")
        segments = path.split("/")
        if INTEGER_SEGMENT not in segments or any(
            ("{" in segment or "}" in segment) and segment != INTEGER_SEGMENT
            for segment in segments
        ):
            raise ValueError("template requires complete integer segments")
        # Validate the rule with a concrete segment while retaining its spelling.
        concrete = "/".join(
            "0" if segment == INTEGER_SEGMENT else segment for segment in segments
        )
        _, query = request_target(concrete + separator + query)
    else:
        path, query = request_target(target)
    return scheme, host, DEFAULT_PORTS[scheme], path, query


@dataclass(frozen=True)
class URLRule:
    url: str
    scheme: str
    host: str
    port: int
    methods: tuple[str, ...]
    path: str
    query_suffix: str
    match: str
    query_mode: str

    @classmethod
    def from_dict(cls, value: dict) -> URLRule:
        if (
            not isinstance(value, dict)
            or not {"url", "methods"} <= value.keys()
            or not value.keys() <= {"url", "methods", "match", "query"}
        ):
            raise ValueError("unexpected URL rule fields")
        methods = value["methods"]
        if (
            not isinstance(methods, list)
            or not methods
            or any(
                not isinstance(method, str) or method not in METHODS
                for method in methods
            )
            or len(set(methods)) != len(methods)
        ):
            raise ValueError("explicit supported HTTP methods are required")
        match = value.get("match", "exact")
        query_mode = value.get("query", "exact")
        if match not in ("exact", "prefix", "template") or query_mode not in (
            "exact",
            "any",
        ):
            raise ValueError("unknown URL matching mode")
        scheme, host, port, path, query = parse_url(
            value["url"], template=match == "template"
        )
        if match == "prefix" and not path.endswith("/"):
            raise ValueError("URL prefix must end with a slash")
        if query_mode == "any" and query not in ("", "?"):
            raise ValueError("any-query rule must not configure a query")
        return cls(
            value["url"],
            scheme,
            host,
            port,
            tuple(methods),
            path,
            query,
            match,
            query_mode,
        )

    def to_dict(self) -> dict:
        return {
            "url": self.url,
            "methods": list(self.methods),
            "match": self.match,
            "query": self.query_mode,
        }

    def matches_path(self, path: str) -> bool:
        if self.match == "exact":
            return path == self.path
        if self.match == "prefix":
            return path.startswith(self.path)
        if self.match != "template":
            return False
        expected = self.path.split("/")
        actual = path.split("/")
        return len(actual) == len(expected) and all(
            bool(actual_segment)
            and actual_segment.isascii()
            and actual_segment.isdecimal()
            if expected_segment == INTEGER_SEGMENT
            else actual_segment == expected_segment
            for expected_segment, actual_segment in zip(expected, actual, strict=True)
        )


@dataclass(frozen=True)
class URLPolicy:
    rules: tuple[URLRule, ...]

    @classmethod
    def from_dict(cls, value: dict) -> URLPolicy:
        if (
            not isinstance(value, dict)
            or set(value) != {"rules"}
            or not isinstance(value["rules"], list)
        ):
            raise ValueError("unexpected URL policy fields")
        return cls(tuple(URLRule.from_dict(rule) for rule in value["rules"]))

    @property
    def hosts(self) -> tuple[str, ...]:
        return tuple(sorted({rule.host for rule in self.rules}))

    def to_dict(self) -> dict:
        return {"rules": [rule.to_dict() for rule in self.rules]}

    def permits(
        self, scheme: str, host: str, port: int, method: str, target: str
    ) -> bool:
        if (
            not isinstance(scheme, str)
            or scheme not in DEFAULT_PORTS
            or type(port) is not int
            or port != DEFAULT_PORTS[scheme]
            or not isinstance(method, str)
            or method not in METHODS
        ):
            return False
        try:
            host = canonical_hostname(host)
            path, query = request_target(target)
        except ValueError:
            return False
        return any(
            rule.scheme == scheme
            and rule.host == host
            and rule.port == port
            and method in rule.methods
            and rule.matches_path(path)
            and (rule.query_mode == "any" or query == rule.query_suffix)
            for rule in self.rules
        )


def profile_config(path: Path, name: str) -> dict:
    document = json.loads(Path(path).read_text())
    if (
        not isinstance(document, dict)
        or set(document) != {"version", "profiles"}
        or type(document["version"]) is not int
        or document["version"] != 1
        or not isinstance(document["profiles"], dict)
        or not isinstance(name, str)
        or name not in document["profiles"]
    ):
        raise ValueError("invalid URL policy bundle or profile")
    profile = document["profiles"][name]
    if (
        not isinstance(profile, dict)
        or "rules" not in profile
        or not profile.keys() <= {"rules", "runner_services"}
        or type(profile.get("runner_services", False)) is not bool
    ):
        raise ValueError("unexpected URL profile fields")
    policy = URLPolicy.from_dict({"rules": profile["rules"]})
    return {
        "rules": policy.to_dict()["rules"],
        "runner_services": profile.get("runner_services", False),
    }


def load(path: Path, name: str) -> URLPolicy:
    profile = profile_config(path, name)
    return URLPolicy.from_dict({"rules": profile["rules"]})
