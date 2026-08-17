"""Run the cache-denial action hooks using only the Python standard library."""

from __future__ import annotations

import http.client
import ipaddress
import json
import os
import secrets
import socket
import ssl
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import SplitResult, urlencode, urlsplit, urlunsplit

CACHE_URL = "https://artifactcache.actions.githubusercontent.com"
RESULTS_URL = "https://results-receiver.actions.githubusercontent.com"
CACHE_READ = (
    "/twirp/github.actions.results.api.v1.CacheService/GetCacheEntryDownloadURL"
)
MAX_RESPONSE = 1024 * 1024


class ActionError(Exception):
    """An error whose message is safe to include in the workflow log."""


@dataclass(frozen=True)
class Runner:
    directory: Path
    certificate: Path
    command: tuple[str, ...]
    installer: str

    def invoke(self, operation: str, *arguments: Path) -> None:
        try:
            subprocess.run(
                [
                    *self.command,
                    str(Path(__file__).with_name(self.installer)),
                    operation,
                    *map(str, arguments),
                ],
                check=True,
            )
        except (OSError, subprocess.CalledProcessError):
            raise ActionError(f"Cache proxy {operation} failed") from None


def enabled() -> bool:
    value = os.environ.get("INPUT_ENABLED") or "true"
    if value not in ("true", "false"):
        raise ActionError("Invalid enabled input")
    return value == "true"


def runner_platform() -> Runner:
    if sys.platform == "win32":
        directory = Path(os.environ["ProgramData"]) / "uv-cache-proxy"
        return Runner(
            directory, directory / "ca.crt", (sys.executable,), "install-windows.py"
        )
    if sys.platform == "darwin":
        directory = Path("/var/run/uv-cache-proxy")
        return Runner(
            directory, directory / "ca.crt", ("sudo", "python3"), "install-macos.py"
        )
    if sys.platform == "linux":
        return Runner(
            Path("/run/uv-cache-proxy"),
            Path("/usr/local/share/ca-certificates/uv-cache-proxy.crt"),
            ("sudo", "python3"),
            "install.py",
        )
    raise ActionError("Unsupported runner platform")


def service_url(value: str) -> SplitResult:
    try:
        if any(ord(character) < 32 or ord(character) == 127 for character in value):
            raise ValueError
        url = urlsplit(value)
        hostname = url.hostname or ""
        github = (
            url.scheme == "https"
            and hostname.endswith(".actions.githubusercontent.com")
            and all(
                character in "abcdefghijklmnopqrstuvwxyz0123456789.-"
                for character in hostname
            )
            and url.port in (None, 443)
        )
        depot = False
        if url.scheme == "http" and url.port in (977, 978):
            depot = ipaddress.ip_address(hostname) in ipaddress.ip_network("10.0.0.0/8")
        if not (github or depot) or url.username or url.password or url.fragment:
            raise ValueError
        return url
    except ValueError:
        raise ActionError("Unexpected cache service endpoint") from None


def origins_for(endpoints: list[str]) -> dict[str, dict]:
    origins = {}
    for endpoint in endpoints:
        url = service_url(endpoint)
        if url.scheme == "http":
            if sys.platform != "linux":
                raise ActionError("Private cache endpoints require Linux")
            origins[f"{url.hostname}:{url.port}"] = {
                "scheme": "http",
                "port": url.port,
                "listen_port": url.port + 19000,
                "forward_origin": urlsplit(
                    RESULTS_URL if url.port == 978 else CACHE_URL
                ).hostname,
                "addresses": [url.hostname],
            }
        elif url.hostname not in origins:
            try:
                addresses = list(
                    dict.fromkeys(
                        address[4][0]
                        for address in socket.getaddrinfo(
                            url.hostname, 443, type=socket.SOCK_STREAM
                        )
                    )
                )
            except OSError:
                addresses = []
            if not addresses:
                raise ActionError("Could not resolve cache service origin")
            origins[url.hostname] = {"addresses": addresses}
    return origins


def request(
    url: SplitResult,
    certificate: Path,
    *,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    body: bytes | None = None,
) -> tuple[int, str | None]:
    """Make a bounded request without redirects, proxy variables, or response logs."""
    connection = None
    try:
        if url.scheme == "https":
            connection = http.client.HTTPSConnection(
                url.hostname,
                url.port,
                timeout=10,
                context=ssl.create_default_context(cafile=str(certificate)),
            )
        else:
            connection = http.client.HTTPConnection(url.hostname, url.port, timeout=10)
        path = urlunsplit(("", "", url.path or "/", url.query, ""))
        connection.request(method, path, body=body, headers=headers or {})
        response = connection.getresponse()
        if len(response.read(MAX_RESPONSE + 1)) > MAX_RESPONSE:
            raise ActionError("Cache service response too large")
        return response.status, response.getheader("X-UV-Cache-Proxy")
    except (OSError, ValueError, http.client.HTTPException):
        raise ActionError("Cache service request failed") from None
    finally:
        if connection is not None:
            connection.close()


def append_variable(file: str, name: str, value: str) -> None:
    if any(character in value for character in "\r\n"):
        raise ActionError("Unsafe environment value")
    with Path(os.environ[file]).open("a", encoding="utf-8") as output:
        output.write(f"{name}={value}\n")


def check_denial(runner: Runner) -> None:
    health = service_url(os.environ["ACTIONS_RESULTS_URL"])._replace(
        path="/__uv_cache_proxy_health", query=""
    )
    for _ in range(20):
        try:
            if request(health, runner.certificate)[0] == 200:
                break
        except ActionError:
            pass
        time.sleep(0.25)
    else:
        raise ActionError("Cache proxy failed its health check")

    key = f"uv-release-cache-check-{uuid.uuid4()}"
    version = secrets.token_hex(32)
    body = json.dumps({"key": key, "version": version, "restore_keys": []}).encode()
    authorization = {"Authorization": f"Bearer {os.environ['ACTIONS_RUNTIME_TOKEN']}"}
    for endpoint in dict.fromkeys([os.environ["ACTIONS_RESULTS_URL"], RESULTS_URL]):
        url = service_url(endpoint)._replace(path=CACHE_READ, query="")
        if request(
            url,
            runner.certificate,
            method="POST",
            headers={**authorization, "Content-Type": "application/json"},
            body=body,
        ) != (403, "denied"):
            raise ActionError("Cache v2 denial self-test failed")

    legacy = service_url(os.environ["ACTIONS_CACHE_URL"])
    legacy = legacy._replace(
        path=legacy.path.rstrip("/") + "/_apis/artifactcache/cache",
        query=urlencode({"keys": key, "version": version}),
    )
    if request(legacy, runner.certificate, headers=authorization) != (403, "denied"):
        raise ActionError("Legacy cache denial self-test failed")


def pre() -> None:
    if not enabled():
        return
    runner = runner_platform()
    if not all(
        os.environ.get(name)
        for name in (
            "ACTIONS_CACHE_URL",
            "ACTIONS_RESULTS_URL",
            "ACTIONS_RUNTIME_TOKEN",
        )
    ):
        raise ActionError("Runner did not provide the cache service credentials")
    origins = origins_for(
        [
            os.environ["ACTIONS_CACHE_URL"],
            os.environ["ACTIONS_RESULTS_URL"],
            CACHE_URL,
            RESULTS_URL,
        ]
    )
    plan = Path(os.environ["RUNNER_TEMP"]) / "uv-release-cache-origins.json"
    plan.write_text(json.dumps(origins), encoding="utf-8")
    try:
        runner.invoke("install", plan)
        check_denial(runner)
        bypass = ",".join(
            filter(
                None,
                [
                    os.environ.get("no_proxy") or os.environ.get("NO_PROXY", ""),
                    *origins,
                ],
            )
        )
        for name, value in {
            "NODE_EXTRA_CA_CERTS": str(runner.certificate),
            "UV_CACHE_PROXY_ACTIVE": "1",
            "UV_CACHE_PROXY_CONFIG": str(runner.directory / "origins.json"),
            "no_proxy": bypass,
            "NO_PROXY": bypass,
        }.items():
            append_variable("GITHUB_ENV", name, value)
        append_variable("GITHUB_STATE", "installed", "true")
        protection = "DNS interception"
        if sys.platform != "win32":
            protection += " and direct-address filtering"
        print(f"GitHub cache API denial is active ({protection}).")
    except Exception:
        try:
            runner.invoke("cleanup")
        except ActionError:
            print("::error::Cache proxy cleanup failed", file=sys.stderr)
        raise


def main() -> None:
    if enabled() and os.environ.get("UV_CACHE_PROXY_ACTIVE") != "1":
        raise ActionError("Cache-denial pre hook did not complete")


def post() -> None:
    if enabled() and os.environ.get("STATE_installed") == "true":
        runner = runner_platform()
        try:
            audit = json.loads((runner.directory / "audit.json").read_text())
            reads = int(audit.get("cache_read_denied", 0))
            writes = int(audit.get("cache_write_denied", 0))
        finally:
            runner.invoke("cleanup")
        print(
            f"Cache proxy removed. Denied {reads} read requests and {writes} write requests."
        )


def run() -> int:
    try:
        {"pre": pre, "main": main, "post": post}[sys.argv[1]]()
    except ActionError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


def report_unexpected_error(_kind, _error, _traceback) -> None:
    # Unhandled exceptions can contain request details or runtime credentials.
    print("::error::Cache-denial action failed", file=sys.stderr)


if __name__ == "__main__":
    sys.excepthook = report_unexpected_error
    sys.exit(run())
