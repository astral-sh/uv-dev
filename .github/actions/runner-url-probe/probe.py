"""Harmless hosted proof of exact-URL enforcement; never logs credentials or bodies."""

import base64
import http.client
import ipaddress
import json
import os
import socket
import ssl
import subprocess
import sys
import time
from pathlib import Path
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

HOST = "api.github.com"
ALLOWED = "/repos/astral-sh/uv-dev"
DENIED = ALLOWED + "/readme"
RATE_LIMIT = "/rate_limit"
MARKER = b"Harmless URL-policy artifact. No executable content.\n"


def api(method, target, address=None):
    context = ssl.create_default_context()
    connection = http.client.HTTPSConnection(HOST, timeout=10, context=context)
    if address is not None:
        connection.sock = context.wrap_socket(
            socket.create_connection((address, 443), timeout=10), server_hostname=HOST
        )
    try:
        connection.request(
            method,
            target,
            headers={
                "User-Agent": "uv-runner-url-policy-probe",
                "Authorization": "Bearer " + os.environ["GH_TOKEN"],
            },
        )
        response = connection.getresponse()
        result = response.status, response.getheader("X-UV-URL-Policy")
        response.read(1024 * 1024)
        return result
    finally:
        connection.close()


def denied(method, target, address=None):
    if api(method, target, address) != (403, "denied"):
        raise RuntimeError("URL policy did not deny the request")


def health():
    connection = http.client.HTTPConnection("127.0.0.1", 18080, timeout=2)
    try:
        connection.request("GET", "/__uv_network_proxy_health")
        if connection.getresponse().status != 204:
            raise RuntimeError("URL policy health check failed")
    finally:
        connection.close()


def scheme_denied():
    # Do not send credentials over cleartext, even when interception is expected.
    connection = http.client.HTTPConnection(HOST, 80, timeout=5)
    try:
        connection.request("GET", ALLOWED)
        response = connection.getresponse()
        if (response.status, response.getheader("X-UV-URL-Policy")) != (403, "denied"):
            raise RuntimeError("unconfigured URL scheme was accepted")
    finally:
        connection.close()


def cache_rpc_paths_denied():
    probes = (
        (
            os.environ["ACTIONS_RUNTIME_URL"],
            "GET",
            "_apis/artifactcache/cache?keys=uv-url-policy-probe&version=synthetic",
            None,
        ),
        (
            os.environ["ACTIONS_RESULTS_URL"],
            "POST",
            "twirp/github.actions.results.api.v1.CacheService/GetCacheEntryDownloadURL",
            json.dumps(
                {
                    "key": "uv-url-policy-probe-" + os.environ["GITHUB_RUN_ID"],
                    "restore_keys": [],
                    "version": "synthetic",
                }
            ).encode(),
        ),
    )
    for base, method, suffix, body in probes:
        endpoint = urlsplit(base)
        if endpoint.scheme == "https":
            if not endpoint.hostname.endswith(
                ".actions.githubusercontent.com"
            ) or endpoint.port not in {None, 443}:
                raise RuntimeError("unexpected runtime service")
            connection = http.client.HTTPSConnection(endpoint.hostname, timeout=10)
        elif (
            endpoint.scheme == "http"
            and ipaddress.ip_address(endpoint.hostname)
            in ipaddress.ip_network("10.0.0.0/8")
            and endpoint.port == 978
        ):
            connection = http.client.HTTPConnection(endpoint.hostname, 978, timeout=10)
        else:
            raise RuntimeError("unexpected runtime service")
        if (
            endpoint.username
            or endpoint.password
            or endpoint.query
            or endpoint.fragment
        ):
            raise RuntimeError("unexpected runtime service metadata")
        try:
            connection.request(
                method,
                endpoint.path.rstrip("/") + "/" + suffix,
                body,
                headers={
                    "Authorization": "Bearer " + os.environ["ACTIONS_RUNTIME_TOKEN"],
                    "Content-Type": "application/json",
                },
            )
            response = connection.getresponse()
            if (response.status, response.getheader("X-UV-URL-Policy")) != (
                403,
                "denied",
            ):
                raise RuntimeError("cache RPC path was not denied locally")
        finally:
            connection.close()


def control():
    for method, target in (
        ("GET", ALLOWED),
        ("HEAD", ALLOWED),
        ("GET", RATE_LIMIT),
        ("GET", DENIED),
    ):
        if api(method, target)[0] != 200:
            raise RuntimeError("repository API control failed")
    addresses = []
    for address in dict.fromkeys(
        item[4][0] for item in socket.getaddrinfo(HOST, 443, type=socket.SOCK_STREAM)
    ):
        if not ipaddress.ip_address(address).is_global:
            continue
        try:
            if api("GET", ALLOWED, address)[0] == 200:
                addresses.append(address)
        except OSError:
            continue
    if not any(ipaddress.ip_address(address).version == 4 for address in addresses):
        raise RuntimeError("direct-address API control failed")
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        output.write("addresses=" + json.dumps(addresses) + "\n")
    (Path(os.environ["RUNNER_TEMP"]) / "url-policy-marker.txt").write_bytes(MARKER)
    print(
        "URL_POLICY_RESULT="
        + json.dumps({"control": True, "addresses": len(addresses)})
    )


def early():
    if os.environ["INPUT_OPERATION"] == "control":
        return
    if os.environ.get("UV_NETWORK_URL_PROFILE") != "github-api-probe":
        raise RuntimeError("URL policy did not run before the later pre hook")
    if api("GET", ALLOWED)[0] != 200:
        raise RuntimeError("allowed HTTPS URL failed in later pre hook")
    denied("GET", DENIED)
    print("URL_POLICY_RESULT=" + json.dumps({"later_pre_denied": True}))


def oidc():
    endpoint = urlsplit(os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"])
    if (
        endpoint.scheme != "https"
        or not endpoint.hostname.endswith(".actions.githubusercontent.com")
        or endpoint.port not in {None, 443}
        or endpoint.username is not None
        or endpoint.password is not None
        or endpoint.fragment
    ):
        raise RuntimeError("unexpected OIDC endpoint")
    query = [
        (name, value)
        for name, value in parse_qsl(endpoint.query, keep_blank_values=True)
        if name != "audience"
    ]
    query.append(("audience", "uv-runner-url-policy-probe"))
    target = urlunsplit(("", "", endpoint.path, urlencode(query), ""))
    connection = http.client.HTTPSConnection(endpoint.hostname, timeout=10)
    try:
        connection.request(
            "GET",
            target,
            headers={
                "Authorization": "Bearer "
                + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]
            },
        )
        response = connection.getresponse()
        payload = json.loads(response.read(65536))
        if response.status != 200 or not isinstance(payload.get("value"), str):
            raise RuntimeError("OIDC control failed")
        parts = payload["value"].split(".")
        if len(parts) != 3:
            raise RuntimeError("OIDC token format differed")
        claims = json.loads(
            base64.urlsafe_b64decode(parts[1] + "=" * (-len(parts[1]) % 4))
        )
        if (
            claims.get("iss") != "https://token.actions.githubusercontent.com"
            or claims.get("aud") != "uv-runner-url-policy-probe"
            or claims.get("repository") != os.environ["GITHUB_REPOSITORY"]
            or claims.get("run_id") != os.environ["GITHUB_RUN_ID"]
        ):
            raise RuntimeError("OIDC claims differed")
    finally:
        connection.close()


def fault(address):
    subprocess.run(
        ["sudo", "-n", "systemctl", "stop", "uv-network-policy.service"], check=True
    )
    try:
        try:
            api("GET", ALLOWED, address)
        except OSError:
            pass
        else:
            raise RuntimeError("stopped proxy allowed direct egress")
    finally:
        subprocess.run(
            ["sudo", "-n", "systemctl", "start", "uv-network-policy.service"],
            check=True,
        )
    for _ in range(20):
        try:
            health()
            break
        except OSError:
            time.sleep(0.1)
    else:
        raise RuntimeError("restarted proxy did not become healthy")
    if api("GET", ALLOWED)[0] != 200:
        raise RuntimeError("restarted proxy did not forward allowed URL")


def probe():
    health()
    for method, target in (("GET", ALLOWED), ("HEAD", ALLOWED), ("GET", RATE_LIMIT)):
        if api(method, target)[0] != 200:
            raise RuntimeError("allowed repository API URL failed")
    denied("GET", DENIED)
    denied("GET", ALLOWED + "?unexpected=1")
    denied("OPTIONS", ALLOWED)
    for target in (
        "/repos//astral-sh/uv-dev",
        "/repos/astral-sh/./uv-dev",
        "/repos/astral-sh/unused/../uv-dev",
        "/repos/astral-sh/%75v-dev",
    ):
        denied("GET", target)
    scheme_denied()
    cache_rpc_paths_denied()
    addresses = json.loads(os.environ["URL_POLICY_BASELINE_ADDRESSES"])
    if not addresses or any(
        not ipaddress.ip_address(address).is_global for address in addresses
    ):
        raise RuntimeError("invalid direct-address controls")
    for address in addresses:
        if api("GET", ALLOWED, address)[0] != 200:
            raise RuntimeError("allowed direct-address request failed")
        denied("GET", DENIED, address)
    connection = http.client.HTTPConnection("127.0.0.1", 18080, timeout=5)
    try:
        connection.request("CONNECT", "denied.invalid:443")
        response = connection.getresponse()
        if (response.status, response.getheader("X-UV-URL-Policy")) != (403, "denied"):
            raise RuntimeError("unconfigured CONNECT was accepted")
    finally:
        connection.close()
    oidc()
    if (
        Path(os.environ["RUNNER_TEMP"]) / "url-policy-seed/url-policy-marker.txt"
    ).read_bytes() != MARKER:
        raise RuntimeError("artifact download differed")
    result = {
        "urls": True,
        "methods": True,
        "queries": True,
        "schemes": True,
        "canonical_paths": True,
        "cache_rpc_paths": True,
        "direct_addresses": len(addresses),
        "oidc": True,
        "artifact": True,
    }
    if os.environ["INPUT_OPERATION"] == "fault":
        fault(addresses[0])
        result["stopped_proxy_denied"] = True
    destination = Path(os.environ["RUNNER_TEMP"]) / "url-policy-evidence.json"
    destination.write_text(json.dumps(result) + "\n")
    print("URL_POLICY_RESULT=" + json.dumps(result))


def main():
    if sys.argv[1] == "early":
        early()
    elif os.environ["INPUT_OPERATION"] == "control":
        control()
    elif os.environ["INPUT_OPERATION"] in {"probe", "fault"}:
        probe()
    else:
        raise ValueError("unknown probe operation")


if __name__ == "__main__":
    try:
        main()
    except (
        OSError,
        ValueError,
        KeyError,
        RuntimeError,
        http.client.HTTPException,
        subprocess.CalledProcessError,
    ):
        print("::error::URL policy acceptance probe failed", file=sys.stderr)
        raise SystemExit(1) from None
