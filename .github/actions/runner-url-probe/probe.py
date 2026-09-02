"""Harmless hosted proof of exact-URL enforcement; never logs credentials or bodies."""

import base64
import errno
import grp
import http.client
import importlib
import ipaddress
import json
import os
import pwd
import socket
import ssl
import stat
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
POLICY_DIRECTORY = Path("/run/uv-network-policy")
PROXY_USER = "uv-network-policy"
CANONICAL_SUDO = Path("/usr/bin/sudo")


def stage(name):
    print("URL_POLICY_STAGE=" + name, flush=True)


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
    status, marker = api(method, target, address)
    if (status, marker) != (403, "denied"):
        print(
            "URL_POLICY_HTTP="
            + json.dumps({"status": status, "local_denial": marker == "denied"}),
            flush=True,
        )
        raise RuntimeError("URL policy did not deny the request")


def health():
    connection = http.client.HTTPConnection("127.0.0.1", 18080, timeout=2)
    try:
        connection.request("GET", "/__uv_network_proxy_health")
        if connection.getresponse().status != 204:
            raise RuntimeError("URL policy health check failed")
    finally:
        connection.close()


def runner_access(path, mode):
    return os.access(path, mode) or os.access(path, mode, effective_ids=True)


def trusted_metadata(path, *, directory=False):
    metadata = path.lstat()
    expected_type = stat.S_ISDIR if directory else stat.S_ISREG
    if (
        not expected_type(metadata.st_mode)
        or metadata.st_uid != 0
        or stat.S_IMODE(metadata.st_mode) & 0o022
        or runner_access(path, os.W_OK)
    ):
        raise RuntimeError("installed proxy metadata is not protected")
    return metadata


def installation_permissions(operation):
    if operation not in {"probe", "fault"} or 0 in {os.getuid(), os.geteuid()}:
        raise RuntimeError("permission checks require the non-root runner")

    stage("trusted-files")
    for directory in (POLICY_DIRECTORY, *POLICY_DIRECTORY.parents):
        trusted_metadata(directory, directory=True)
    for name in ("policy.py", "proxy.py", "url_policy.py", "url_proxy.py"):
        trusted_metadata(POLICY_DIRECTORY / name)
    for name in ("policies.json", "url-policy.json", "settings.json", "rules.nft"):
        trusted_metadata(POLICY_DIRECTORY / name)
    trusted_metadata(Path("/run/systemd/system/uv-network-policy.service"))

    stage("proxy-group")
    account = pwd.getpwnam(PROXY_USER)
    group = grp.getgrnam(PROXY_USER)
    runner_groups = {*os.getgroups(), os.getgid(), os.getegid()}
    if (
        account.pw_name != PROXY_USER
        or group.gr_name != PROXY_USER
        or account.pw_uid in {0, os.getuid(), os.geteuid()}
        or account.pw_gid == 0
        or account.pw_gid != group.gr_gid
        or account.pw_gid in runner_groups
        or any(member != PROXY_USER for member in group.gr_mem)
        or any(
            item.pw_gid == account.pw_gid and item.pw_name != PROXY_USER
            for item in pwd.getpwall()
        )
        or any(
            item.gr_gid == group.gr_gid
            and (
                item.gr_name != PROXY_USER
                or any(member != PROXY_USER for member in item.gr_mem)
            )
            for item in grp.getgrall()
        )
    ):
        raise RuntimeError("proxy primary group is not dedicated")

    stage("tls-key")
    key = POLICY_DIRECTORY / "server.key"
    metadata = key.lstat()
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != 0
        or metadata.st_gid != group.gr_gid
        or stat.S_IMODE(metadata.st_mode) != 0o640
    ):
        raise RuntimeError("TLS key ownership or mode differed")
    # Check access through the kernel; never open or read the private key.
    if runner_access(key, os.R_OK):
        raise RuntimeError("runner can directly read the TLS key")

    stage("sudo")
    sudo = CANONICAL_SUDO.resolve(strict=True)
    metadata = trusted_metadata(sudo)
    harmless = Path("/usr/bin/true").resolve(strict=True)
    trusted_metadata(harmless)
    if operation == "probe" and (
        stat.S_IMODE(metadata.st_mode) != 0o700
        or runner_access(sudo, os.R_OK)
        or runner_access(sudo, os.X_OK)
    ):
        raise RuntimeError("canonical sudo is not root-only")
    try:
        completed = subprocess.run(
            [str(sudo), "-n", "--", str(harmless)],
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
        )
    except PermissionError as error:
        if operation != "probe" or error.errno != errno.EACCES:
            raise RuntimeError("unexpected canonical sudo access failure") from None
    except subprocess.TimeoutExpired:
        raise RuntimeError("canonical sudo check timed out") from None
    else:
        if operation != "fault" or completed.returncode != 0:
            raise RuntimeError("canonical sudo privilege check differed")

    return {
        "trusted_proxy_code": True,
        "trusted_effective_policy": True,
        "dedicated_proxy_group": True,
        "private_tls_key": True,
        "runner_key_unreadable": True,
        "sudo_restricted": operation == "probe",
        "sudo_retained": operation == "fault",
    }


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
    probes = [
        (
            "cache-legacy",
            os.environ["ACTIONS_RUNTIME_URL"],
            "GET",
            "_apis/artifactcache/cache?keys=uv-url-policy-probe&version=synthetic",
            None,
        ),
        (
            "cache-v2",
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
    ]
    cache_url = os.environ.get("ACTIONS_CACHE_URL", "")
    private_legacy = urlsplit(cache_url).scheme == "http"
    if private_legacy:
        probes.append(
            (
                "cache-depot-legacy",
                cache_url,
                "GET",
                "_apis/artifactcache/cache?keys=uv-url-policy-probe&version=synthetic",
                None,
            )
        )
    for label, base, method, suffix, body in probes:
        stage(label)
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
            and endpoint.port in {977, 978}
        ):
            connection = http.client.HTTPConnection(
                endpoint.hostname, endpoint.port, timeout=10
            )
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
    return private_legacy


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
    if os.environ["INPUT_OPERATION"] in {"control", "metadata"}:
        return
    if os.environ.get("UV_NETWORK_URL_PROFILE") != "github-api-probe":
        raise RuntimeError("URL policy did not run before the later pre hook")
    if api("GET", ALLOWED)[0] != 200:
        raise RuntimeError("allowed HTTPS URL failed in later pre hook")
    denied("GET", DENIED)
    print("URL_POLICY_RESULT=" + json.dumps({"later_pre_denied": True}))


def metadata():
    endpoint = urlsplit(os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"])
    result = {
        "https": endpoint.scheme == "https",
        "actions_host": bool(endpoint.hostname)
        and endpoint.hostname.endswith(".actions.githubusercontent.com"),
        "default_port": endpoint.port in {None, 443},
        "path_is_root": endpoint.path in {"", "/"},
        "path_length": len(endpoint.path),
        "repeated_slash": "//" in endpoint.path,
        "dot_segment": any(part in {".", ".."} for part in endpoint.path.split("/")),
        "semicolon": ";" in endpoint.path,
        "percent_escape": "%" in endpoint.path,
        "braces": any(part in endpoint.path for part in "{}"),
        "query_present": bool(endpoint.query),
        "fragment_present": bool(endpoint.fragment),
    }
    source = (
        Path(os.environ["GITHUB_WORKSPACE"]) / ".github/actions/runner-network-policy"
    )
    sys.path.insert(0, str(source))
    settings = importlib.import_module("url_settings")
    domain = importlib.import_module("policy")
    phase = "runtime-metadata"
    try:
        services = settings.runtime_services(os.environ)
        phase = "url-profile"
        settings.compile_policy(
            source / "url-policies.json",
            "github-api-probe",
            domain.load(source / "policies.json", "github"),
            services,
        )
        result["configuration_accepted"] = True
    except (ValueError, TypeError, KeyError) as error:
        reasons = {
            "invalid OIDC service URL",
            "unexpected OIDC service URL",
            "invalid runner service URL",
            "unexpected runner service URL",
            "URL must use visible ASCII",
            "invalid URL character",
            "invalid URL escape",
            "ambiguous URL escape",
            "ambiguous request path",
            "URL profile exceeds its domain policy",
            "too many URL policy hosts",
        }
        result["configuration_accepted"] = False
        result["rejected_at"] = phase
        result["reason"] = str(error) if str(error) in reasons else type(error).__name__
    print("URL_POLICY_METADATA=" + json.dumps(result), flush=True)


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
        body = response.read(65536)
        if response.status != 200:
            print(
                "URL_POLICY_OIDC_HTTP="
                + json.dumps(
                    {
                        "status": response.status,
                        "local_denial": response.getheader("X-UV-URL-Policy")
                        == "denied",
                    }
                ),
                flush=True,
            )
            raise RuntimeError("OIDC control failed")
        payload = json.loads(body)
        if not isinstance(payload, dict):
            raise TypeError("OIDC response format differed")
        if not isinstance(payload.get("value"), str):
            raise TypeError("OIDC token format differed")
        parts = payload["value"].split(".")
        if len(parts) != 3:
            raise RuntimeError("OIDC token format differed")
        claims = json.loads(
            base64.urlsafe_b64decode(parts[1] + "=" * (-len(parts[1]) % 4))
        )
        if not isinstance(claims, dict):
            raise TypeError("OIDC claims format differed")
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
        [
            str(CANONICAL_SUDO),
            "-n",
            "--",
            "/usr/bin/systemctl",
            "stop",
            "uv-network-policy.service",
        ],
        check=True,
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
            [
                str(CANONICAL_SUDO),
                "-n",
                "--",
                "/usr/bin/systemctl",
                "start",
                "uv-network-policy.service",
            ],
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
    stage("health")
    health()
    permissions = installation_permissions(os.environ["INPUT_OPERATION"])
    stage("allowed-urls")
    for method, target in (("GET", ALLOWED), ("HEAD", ALLOWED), ("GET", RATE_LIMIT)):
        if api(method, target)[0] != 200:
            raise RuntimeError("allowed repository API URL failed")
    stage("path-method-query-denials")
    denied("GET", DENIED)
    denied("GET", ALLOWED + "?unexpected=1")
    denied("OPTIONS", ALLOWED)
    stage("canonical-paths")
    for target in (
        "/repos//astral-sh/uv-dev",
        "/repos/astral-sh/./uv-dev",
        "/repos/astral-sh/unused/../uv-dev",
        "/repos/astral-sh/%75v-dev",
    ):
        denied("GET", target)
    stage("scheme")
    scheme_denied()
    private_legacy = cache_rpc_paths_denied()
    stage("direct-addresses")
    addresses = json.loads(os.environ["URL_POLICY_BASELINE_ADDRESSES"])
    if not addresses or any(
        not ipaddress.ip_address(address).is_global for address in addresses
    ):
        raise RuntimeError("invalid direct-address controls")
    for address in addresses:
        if api("GET", ALLOWED, address)[0] != 200:
            raise RuntimeError("allowed direct-address request failed")
        denied("GET", DENIED, address)
    stage("connect")
    connection = http.client.HTTPConnection("127.0.0.1", 18080, timeout=5)
    try:
        connection.request(
            "CONNECT", "denied.invalid:443", headers={"Host": "denied.invalid:443"}
        )
        response = connection.getresponse()
        if (response.status, response.getheader("X-UV-URL-Policy")) != (403, "denied"):
            raise RuntimeError("unconfigured CONNECT was accepted")
    finally:
        connection.close()
    stage("oidc")
    oidc()
    stage("artifact")
    if (
        Path(os.environ["RUNNER_TEMP"]) / "url-policy-seed/url-policy-marker.txt"
    ).read_bytes() != MARKER:
        raise RuntimeError("artifact download differed")
    result = {
        **permissions,
        "urls": True,
        "methods": True,
        "queries": True,
        "schemes": True,
        "canonical_paths": True,
        "cache_rpc_paths": True,
        "private_cache_entrypoint": private_legacy,
        "direct_addresses": len(addresses),
        "oidc": True,
        "artifact": True,
    }
    if os.environ["INPUT_OPERATION"] == "fault":
        stage("stopped-proxy")
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
    elif os.environ["INPUT_OPERATION"] == "metadata":
        metadata()
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
        TypeError,
        KeyError,
        RuntimeError,
        http.client.HTTPException,
        subprocess.CalledProcessError,
    ) as error:
        print(
            f"::error::URL policy acceptance probe failed ({type(error).__name__})",
            file=sys.stderr,
        )
        raise SystemExit(1) from None
