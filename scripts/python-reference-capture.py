"""Capture four paired uv and curl downloads of one managed-Python archive."""

import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
import platform
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.request
import zipfile
from pathlib import Path
from urllib.parse import urlsplit

from python_reference_tls import BODY, Server
from windows_process_job import run_process_tree

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "address_capture", HERE / "python-address-capture.py"
)
assert SPEC is not None and SPEC.loader is not None
common = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(common)

BASE_REVISION = "dca8380afc8348075785a366d96bee6ac7934efe"
HOST = "releases.astral.sh"
PEER = "104.26.13.77:443"
ARCHIVE_SIZE = 24417043
ARCHIVE_URL = (
    common.MIRROR
    + "/20261013/cpython-3.15.0%2B20261013-x86_64-pc-windows-msvc-install_only_stripped.tar.gz"
)
CURL_VERSION = "8.23.0"
CURL_PACKAGE = "curl-8.23.0_1-win64-mingw"
CURL_URL = f"https://curl.se/windows/dl-8.23.0_1/{CURL_PACKAGE}.zip"
CURL_SHA256 = "752ff19c45be64af5b5f8d5222dce64e0f7b03d740a7362a4055c220fa9637c6"
SEQUENCES = {1: ["uv", "curl", "uv", "curl"], 2: ["curl", "uv", "curl", "uv"]}
PROXY_NAMES = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
]
CURL_FIELDS = [
    "http_code",
    "http_version",
    "remote_ip",
    "remote_port",
    "url_effective",
    "size_download",
    "num_connects",
    "num_redirects",
    "num_retries",
    "exitcode",
    "time_namelookup",
    "time_connect",
    "time_appconnect",
    "time_pretransfer",
    "time_starttransfer",
    "time_total",
    "ssl_verify_result",
]
HEADER_NAMES = {
    "age",
    "cf-cache-status",
    "cf-ray",
    "content-length",
    "content-encoding",
    "etag",
}
SOURCE_FILES = common.SOURCE_FILES + [
    "scripts/python-reference-capture.py",
    "scripts/python_reference_tls.py",
    "scripts/python-reference-certificates/certificate.pem",
    "scripts/python-reference-certificates/private-key.pem",
]


def write_json(path, value):
    common.write_json(path, value)


def prepare_curl(directory, archive=None):
    directory.mkdir(parents=True, exist_ok=False)
    target = directory / f"{CURL_PACKAGE}.zip"
    record = {"url": CURL_URL, "expected_sha256": CURL_SHA256, "status": "preparing"}
    write_json(directory / "provenance.json", record)
    try:
        if archive is None:

            class RejectRedirects(urllib.request.HTTPRedirectHandler):
                def redirect_request(
                    self, _request, _file, _code, _message, _headers, _url
                ):
                    raise ValueError("The pinned curl download redirected")

            opener = urllib.request.build_opener(RejectRedirects())
            with (
                opener.open(CURL_URL, timeout=30) as response,
                target.open("wb") as output,
            ):
                if response.url != CURL_URL:
                    raise ValueError("The pinned curl download redirected")
                started = time.monotonic()
                count = 0
                while chunk := response.read(1024 * 1024):
                    count += len(chunk)
                    if count > 32 * 1024 * 1024 or time.monotonic() - started > 120:
                        raise ValueError("The curl package exceeded its download bound")
                    output.write(chunk)
        else:
            shutil.copyfile(archive, target)
        record["archive_sha256"] = common.digest(target)
        if record["archive_sha256"] != CURL_SHA256:
            raise ValueError("The curl package differs from its official checksum")
        with zipfile.ZipFile(target) as package:
            if package.testzip() is not None:
                raise ValueError("The curl package failed its CRC check")
            record["files"] = {}
            for name in ["curl.exe", "curl-ca-bundle.crt"]:
                data = package.read(f"{CURL_PACKAGE}/bin/{name}")
                (directory / name).write_bytes(data)
                record["files"][name] = {
                    "sha256": hashlib.sha256(data).hexdigest(),
                    "size_bytes": len(data),
                }
        if not (directory / "curl.exe").read_bytes().startswith(b"MZ"):
            raise ValueError("The pinned curl executable is not a Windows binary")
        record["status"] = "verified_package"
    except (
        OSError,
        ValueError,
        RuntimeError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
    ) as error:
        record.update(status="error", error_type=type(error).__name__, error=str(error))
        raise
    finally:
        write_json(directory / "provenance.json", record)
    return record


def curl_arguments(binary, ca_bundle, url, peer, body, headers, request_headers):
    parsed = urlsplit(url)
    port = parsed.port or 443
    address = peer.rsplit(":", 1)[0]
    arguments = [
        str(binary),
        "--disable",
        "--silent",
        "--show-error",
        "--http2",
        "--proto",
        "=https",
        "--connect-timeout",
        "10",
        "--max-time",
        "120",
        "--retry",
        "0",
        "--max-redirs",
        "0",
        "--cacert",
        str(ca_bundle),
        "--resolve",
        f"{parsed.hostname}:{port}:{address}",
        "--user-agent",
        request_headers["user-agent"],
        "--header",
        "Accept: " + request_headers["accept"],
        "--header",
        "Accept-Encoding: " + request_headers["accept-encoding"],
        "--compressed",
        "--output",
        str(body),
        "--dump-header",
        str(headers),
        "--write-out",
        "%{json}\n",
        url,
    ]
    return arguments


def run_single(arguments, environment, directory, deadline=20):
    directory.mkdir(parents=True, exist_ok=False)
    stdout, stderr = directory / "stdout.log", directory / "stderr.log"
    if os.name == "nt":
        return run_process_tree(
            arguments, environment, directory, stdout, stderr, deadline
        )
    started = time.monotonic()
    with stdout.open("wb") as output, stderr.open("wb") as errors:
        process = subprocess.run(
            arguments,
            env=environment,
            cwd=directory,
            stdout=output,
            stderr=errors,
            timeout=deadline,
            check=False,
        )
    return {
        "exit_code": process.returncode,
        "elapsed_seconds": time.monotonic() - started,
        "tree_exit_confirmed": True,
        "platform_limit": "Linux root process only; Windows Job Object was not executed",
    }


def local_environment():
    environment = dict(os.environ)
    environment.update(
        NO_PROXY="releases.astral.sh,wrong-host.invalid,127.0.0.1",
        no_proxy="releases.astral.sh,wrong-host.invalid,127.0.0.1",
    )
    return environment


def tls_preflights(uv, curl, output, workspace):
    output.mkdir()
    workspace.mkdir()
    certificate = HERE / "python-reference-certificates/certificate.pem"
    private_key = HERE / "python-reference-certificates/private-key.pem"
    record = {
        "status": "running",
        "included_in_live_observations": False,
        "platform": platform.platform(),
        "checks": [],
    }
    write_json(output / "result.json", record)

    def uv_fixture(name, protocol, *, refuse_stream=False):
        server = Server(certificate, private_key, protocol, refuse_stream=refuse_stream)
        work = workspace / name
        work.mkdir()
        for child in ["archives", "install", "cache", "bin", "tmp", "traces"]:
            (work / child).mkdir()
        environment, _ = common.trial_environment(work, work / "traces", "B")
        environment.update(
            NO_PROXY="releases.astral.sh,127.0.0.1",
            no_proxy="releases.astral.sh,127.0.0.1",
        )
        environment.update(
            {
                "UV_INTERNAL__TEST_RELEASES_ADDRESS": "127.0.0.1:0",
                "UV_PYTHON_INSTALL_MIRROR": f"https://{HOST}:{server.port}/github/python-build-standalone/releases/download",
                "SSL_CERT_FILE": str(certificate),
                "UV_HTTP_TIMEOUT": "5",
                "UV_HTTP_CONNECT_TIMEOUT": "3",
                "RUST_LOG": "error,reqwest::retry=trace",
            }
        )
        result = run_single(common.install_arguments(uv), environment, work / "process")
        observed = server.finish()
        observations = common.trace_observations(work / "traces")
        result.update(name=name, server=observed, traces=observations)
        record["checks"].append(result)
        write_json(output / "result.json", record)
        errors = (work / "process/stderr.log").read_text(errors="replace")
        result["protocol_retry_events"] = [
            line
            for line in errors.splitlines()
            if any(
                message in line
                for message in ["could retry!", "max_retries_per_request hit"]
            )
        ]
        if refuse_stream:
            if (
                result["exit_code"] == 0
                or observed["requests"] != 3
                or observations["response_identities"]
                or not result["protocol_retry_events"]
            ):
                raise ValueError(
                    "The reqwest refused-stream control differed from its pinned policy"
                )
        elif (
            result["exit_code"] != 1
            or "Hash mismatch" not in errors
            or observed.get("sni") != [HOST]
            or observed.get("alpn") != protocol
            or observed["requests"] != 1
        ):
            raise ValueError(f"uv local TLS preflight failed: {name}")
        result["status"] = "passed"
        write_json(output / "result.json", record)
        return observed

    try:
        observed = uv_fixture("uv-http1-request-headers", "http/1.1")
        request_headers = {
            name: observed["request_headers"][name]
            for name in ["user-agent", "accept", "accept-encoding"]
        }
        if any("\r" in value or "\n" in value for value in request_headers.values()):
            raise ValueError("Unexpected request header characters")
        record["matched_request_headers"] = request_headers
        uv_fixture("uv-http2", "h2")
        uv_fixture("uv-refused-stream", "h2", refuse_stream=True)
        for name, protocol, hostname, trusted in [
            ("curl-http1-request-headers", "http/1.1", HOST, True),
            ("curl-http2", "h2", HOST, True),
            ("curl-wrong-hostname", "h2", "wrong-host.invalid", True),
            ("curl-untrusted-certificate", "h2", HOST, False),
        ]:
            server = Server(certificate, private_key, protocol)
            work = workspace / name
            work.mkdir()
            ca = certificate if trusted else curl.parent / "curl-ca-bundle.crt"
            if not ca.is_file():
                ca = Path("/etc/ssl/certs/ca-certificates.crt")
            arguments = curl_arguments(
                curl,
                ca,
                f"https://{hostname}:{server.port}/fixture.tar.gz",
                "127.0.0.1:0",
                work / "body",
                work / "headers",
                request_headers,
            )
            if protocol == "http/1.1":
                arguments[arguments.index("--http2")] = "--http1.1"
            result = run_single(arguments, local_environment(), work / "process")
            observed = server.finish()
            result.update(name=name, server=observed)
            record["checks"].append(result)
            write_json(output / "result.json", record)
            if hostname != HOST or not trusted:
                if result["exit_code"] != 60 or observed["requests"] != 0:
                    raise ValueError(f"curl accepted a rejected TLS fixture: {name}")
            else:
                transfer = json.loads((work / "process/stdout.log").read_text())
                if (
                    result["exit_code"] != 0
                    or (work / "body").read_bytes() != BODY
                    or observed.get("sni") != [HOST]
                    or observed.get("alpn") != protocol
                    or observed["requests"] != 1
                ):
                    raise ValueError(f"curl local TLS preflight failed: {name}")
                if str(transfer["http_version"]) != (
                    "2" if protocol == "h2" else "1.1"
                ):
                    raise ValueError("curl reported a different local protocol")
                if protocol == "http/1.1" and any(
                    observed["request_headers"].get(name) != value
                    for name, value in request_headers.items()
                ):
                    raise ValueError("The client request headers differ")
                result["verified_body_sha256"] = common.digest(work / "body")
                result["transfer"] = {key: transfer.get(key) for key in CURL_FIELDS}
            result["status"] = "passed"
            write_json(output / "result.json", record)
        refused = Server(certificate, private_key, "h2", refuse_stream=True)
        work = workspace / "curl-refused-stream"
        work.mkdir()
        arguments = curl_arguments(
            curl,
            certificate,
            f"https://{HOST}:{refused.port}/fixture.tar.gz",
            "127.0.0.1:0",
            work / "body",
            work / "headers",
            request_headers,
        )
        result = run_single(arguments, local_environment(), work / "process")
        observed = refused.finish()
        transfer = json.loads((work / "process/stdout.log").read_text())
        result.update(
            name="curl-refused-stream",
            server=observed,
            transfer={key: transfer.get(key) for key in CURL_FIELDS},
        )
        record["checks"].append(result)
        write_json(output / "result.json", record)
        if (
            result["exit_code"] == 0
            or observed["requests"] != 6
            or transfer["num_connects"] != 6
        ):
            raise ValueError(
                "The curl refused-stream control differed from its pinned policy"
            )
        result["status"] = "passed"
        proxy = Server(certificate, private_key, proxy=True)
        work = workspace / "curl-configured-proxy"
        work.mkdir()
        environment = local_environment()
        environment.update(
            HTTPS_PROXY=f"http://127.0.0.1:{proxy.port}",
            https_proxy=f"http://127.0.0.1:{proxy.port}",
            NO_PROXY="",
            no_proxy="",
        )
        arguments = curl_arguments(
            curl,
            certificate,
            f"https://{HOST}:9/fixture.tar.gz",
            "127.0.0.1:0",
            work / "body",
            work / "headers",
            request_headers,
        )
        result = run_single(arguments, environment, work / "process")
        observed = proxy.finish()
        if result["exit_code"] == 0 or observed.get("connect_authority") != f"{HOST}:9":
            raise ValueError("curl did not retain the configured proxy")
        record["checks"].append(
            {
                "name": "curl-configured-proxy",
                **result,
                "server": observed,
                "status": "passed",
            }
        )
        record["status"] = "passed"
    except (
        OSError,
        ValueError,
        RuntimeError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
    ) as error:
        record.update(
            status="failed", error_type=type(error).__name__, error=str(error)
        )
        raise
    finally:
        write_json(output / "result.json", record)
    return record


def body_observation(configuration):
    directory = Path(configuration["body_directory"])
    files = [path for path in directory.rglob("*") if path.is_file()]
    return sum(path.stat().st_size for path in files)


def pair_worker(configuration_path):
    configuration = json.loads(configuration_path.read_text())
    result_path = Path(configuration["result_path"])
    result = {
        "status": "starting",
        "clients": {},
        "started_monotonic": time.monotonic(),
        "started_at": common.timestamp(),
    }
    processes = {}
    with contextlib.ExitStack() as handles:
        try:
            for name in configuration["launch_order"]:
                client = configuration["clients"][name]
                environment = dict(os.environ)
                for variable in client.get("environment_remove", []):
                    environment.pop(variable, None)
                environment.update(client.get("environment_overrides", {}))
                stdout = handles.enter_context(Path(client["stdout"]).open("wb"))
                stderr = handles.enter_context(Path(client["stderr"]).open("wb"))
                child = {
                    "launched_at": common.timestamp(),
                    "launch_monotonic": time.monotonic(),
                    "progress": [],
                }
                result["clients"][name] = child
                write_json(result_path, result)
                process = subprocess.Popen(
                    client["arguments"],
                    cwd=client["cwd"],
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                    close_fds=True,
                )
                processes[name] = process
                child.update(pid=process.pid, spawn_returned_monotonic=time.monotonic())
                write_json(result_path, result)
            result["status"] = "running"
            while any(process.poll() is None for process in processes.values()):
                changed = False
                for name, process in processes.items():
                    process.poll()
                    child = result["clients"][name]
                    client = configuration["clients"][name]
                    try:
                        size = body_observation(client)
                        if (
                            not child["progress"]
                            or child["progress"][-1]["bytes_on_disk"] != size
                        ):
                            child["progress"].append(
                                {
                                    "observed_monotonic": time.monotonic(),
                                    "bytes_on_disk": size,
                                }
                            )
                            changed = True
                    except OSError as error:
                        child.setdefault("observation_errors", []).append(
                            type(error).__name__
                        )
                    if process.returncode is not None and "exit_code" not in child:
                        child.update(
                            exit_code=process.returncode,
                            finished_monotonic=time.monotonic(),
                            finished_at=common.timestamp(),
                        )
                        changed = True
                if changed:
                    write_json(result_path, result)
                time.sleep(0.1)
            for name, process in processes.items():
                child = result["clients"][name]
                child.setdefault("exit_code", process.wait())
                child.setdefault("finished_monotonic", time.monotonic())
                child.setdefault("finished_at", common.timestamp())
                child["progress"].append(
                    {
                        "observed_monotonic": time.monotonic(),
                        "bytes_on_disk": body_observation(
                            configuration["clients"][name]
                        ),
                    }
                )
            result["status"] = "completed"
        except (
            OSError,
            ValueError,
            RuntimeError,
            KeyError,
            TypeError,
            subprocess.SubprocessError,
        ) as error:
            result.update(
                status="error", error_type=type(error).__name__, error=str(error)
            )
            write_json(result_path, result)
            # The owning job supplies the deadline while every started child exits.
            for process in processes.values():
                process.wait()
        finally:
            result["finished_monotonic"] = time.monotonic()
            result["finished_at"] = common.timestamp()
            write_json(result_path, result)
    return int(result["status"] != "completed")


def safe_headers(path):
    headers = {}
    if not path.is_file():
        return headers
    for line in path.read_text(errors="replace").splitlines():
        if line.startswith("HTTP/"):
            headers = {}
        elif ":" in line:
            name, value = line.split(":", 1)
            name, value = name.lower(), value.strip()
            if (
                name in HEADER_NAMES
                and len(value) <= 128
                and all(32 <= ord(character) <= 126 for character in value)
            ):
                headers[name] = value
    return headers


def summarize_curl(work, child):
    result = {
        "outcome": "incomplete",
        "process": child,
        "response_headers": safe_headers(work / "headers"),
    }
    output = work / "stdout.log"
    if output.is_file():
        try:
            value = json.loads(output.read_text())
            result["transfer"] = {key: value.get(key) for key in CURL_FIELDS}
        except (ValueError, TypeError):
            result["parse_error"] = "No complete curl transfer JSON"
    transfer = result.get("transfer", {})
    body = work / "body/archive.tar.gz"
    result["body_size"] = body.stat().st_size if body.is_file() else None
    result["body_sha256"] = common.digest(body) if body.is_file() else None
    errors = []
    checks = [
        (child.get("exit_code") == 0, "curl did not exit successfully"),
        (transfer.get("http_code") == 200, "Final response was not HTTP 200"),
        (str(transfer.get("http_version")) == "2", "Final response was not HTTP/2"),
        (
            transfer.get("remote_ip") == PEER.rsplit(":", 1)[0]
            and transfer.get("remote_port") == 443,
            "Socket peer differs",
        ),
        (transfer.get("url_effective") == ARCHIVE_URL, "Final URL differs"),
        (transfer.get("num_redirects") == 0, "A redirect occurred"),
        (transfer.get("num_retries") == 0, "A retry occurred"),
        (transfer.get("num_connects") == 1, "Expected one new connection"),
        (transfer.get("ssl_verify_result") == 0, "TLS verification was not successful"),
        (
            result["body_size"] == ARCHIVE_SIZE
            and result["body_sha256"] == common.EXPECTED_ARCHIVE_SHA256,
            "Archive bytes differ",
        ),
    ]
    for valid, reason in checks:
        if not valid:
            errors.append(reason)
    result["control_errors"] = errors
    result["outcome"] = "success" if not errors else "unmatched_or_failed"
    if isinstance(transfer.get("time_total"), (int, float)) and isinstance(
        transfer.get("time_starttransfer"), (int, float)
    ):
        result["after_first_response_byte_seconds"] = (
            transfer["time_total"] - transfer["time_starttransfer"]
        )
    return result


def uv_phases(directory):
    events = {
        "request_started": [],
        "response_headers": [],
        "copy_started": [],
        "copy_finished": [],
    }
    markers = {
        "request_started": "Sending Python download request",
        "response_headers": "Received Python download response headers",
        "copy_started": "Copying Python archive",
        "copy_finished": "Copied Python archive bytes=",
    }
    statuses = []
    copy_elapsed = []
    for path in sorted(directory.glob("*.log")):
        for line in path.read_text(errors="replace").splitlines():
            timestamp = re.match(r"\s*([0-9.]+)s ", line)
            if timestamp:
                for name, marker in markers.items():
                    if marker in line:
                        events[name].append(float(timestamp[1]))
                if markers["response_headers"] in line:
                    status = re.search(r" status=(\d{3})\b", line)
                    if status:
                        statuses.append(int(status[1]))
            if (
                'outcome="complete"' in line
                and "Python archive copy observation" in line
            ):
                elapsed = re.search(r" elapsed_us=(\d+)\b", line)
                if elapsed:
                    copy_elapsed.append(int(elapsed[1]) / 1_000_000)
    result = {
        "process_relative_event_seconds": events,
        "response_statuses": statuses,
        "completed_body_copy_seconds": copy_elapsed,
    }
    if all(len(values) == 1 for values in events.values()):
        result["request_to_headers_seconds"] = (
            events["response_headers"][0] - events["request_started"][0]
        )
        result["headers_to_copy_finished_seconds"] = (
            events["copy_finished"][0] - events["response_headers"][0]
        )
    return result


def run_pair(
    uv, curl, ca_bundle, request_headers, output, workspace, index, first, archive_name
):
    name = f"pair-{index:02d}-{first}-first"
    evidence, work = output / name, workspace / name
    evidence.mkdir()
    work.mkdir()
    for client in ["uv", "curl"]:
        (work / client).mkdir()
    uv_work, curl_work = work / "uv", work / "curl"
    for child in ["archives", "install", "cache", "bin", "tmp", "child-traces"]:
        (uv_work / child).mkdir()
    (curl_work / "body").mkdir()
    environment, overrides = common.trial_environment(
        uv_work, uv_work / "child-traces", "B"
    )
    overrides["RUST_LOG"] = "error,reqwest::retry=trace"
    removed = sorted(set(os.environ) - set(environment))
    configuration = {
        "result_path": str(work / "worker.json"),
        "launch_order": [first, "curl" if first == "uv" else "uv"],
        "clients": {
            "uv": {
                "arguments": common.install_arguments(uv),
                "environment_overrides": overrides,
                "environment_remove": removed,
                "cwd": str(uv_work),
                "stdout": str(uv_work / "stdout.log"),
                "stderr": str(uv_work / "stderr.log"),
                "body_directory": str(uv_work / "archives"),
            },
            "curl": {
                "arguments": curl_arguments(
                    curl,
                    ca_bundle,
                    ARCHIVE_URL,
                    PEER,
                    curl_work / "body/archive.tar.gz",
                    curl_work / "headers",
                    request_headers,
                ),
                "cwd": str(curl_work),
                "stdout": str(curl_work / "stdout.log"),
                "stderr": str(curl_work / "stderr.log"),
                "body_directory": str(curl_work / "body"),
            },
        },
    }
    write_json(work / "configuration.json", configuration)
    result = {
        "pair": index,
        "launch_order": configuration["launch_order"],
        "started_at": common.timestamp(),
        "status": "running",
        "whole_pair_deadline_seconds": 120,
        "commands": {
            name: client["arguments"]
            for name, client in configuration["clients"].items()
        },
        "uv_environment_overrides": overrides,
    }
    write_json(evidence / "result.json", result)
    try:
        result["supervisor"] = run_process_tree(
            [
                sys.executable,
                str(Path(__file__).resolve()),
                "pair-worker",
                str(work / "configuration.json"),
            ],
            dict(os.environ),
            work,
            work / "stdout.log",
            work / "stderr.log",
            120,
        )
        if not result["supervisor"]["tree_exit_confirmed"]:
            raise RuntimeError("Pair process-tree exit was not confirmed")
        worker = (
            json.loads((work / "worker.json").read_text())
            if (work / "worker.json").is_file()
            else {"status": "missing", "clients": {}}
        )
        result["worker"] = worker
        for client in ["uv", "curl"]:
            destination = evidence / client
            destination.mkdir()
            if client == "uv":
                common.copy_sanitized_logs(work / client, destination)
            else:
                stderr = work / client / "stderr.log"
                if stderr.is_file():
                    (destination / "stderr.log").write_text(
                        common.sanitize_log_text(stderr.read_text(errors="replace")),
                        encoding="utf-8",
                    )
        uv_child = worker["clients"].get("uv", {})
        curl_child = worker["clients"].get("curl", {})
        traces = common.trace_observations(evidence / "uv/child-traces")
        archive = uv_work / "archives" / archive_name
        result["uv"] = {
            "process": uv_child,
            "traces": traces,
            "phases": uv_phases(evidence / "uv/child-traces"),
            "archive_size": archive.stat().st_size if archive.is_file() else None,
            "archive_sha256": common.digest(archive) if archive.is_file() else None,
        }
        uv_errors = common.response_control_errors(traces["response_identities"], "B")
        if (
            uv_child.get("exit_code") != 0
            or traces["managed_download_send_events"] != 1
            or traces["parse_errors"]
            or result["uv"]["phases"]["response_statuses"] != [200]
            or result["uv"]["archive_size"] != ARCHIVE_SIZE
            or result["uv"]["archive_sha256"] != common.EXPECTED_ARCHIVE_SHA256
        ):
            uv_errors.append("uv command, request count or archive integrity differs")
        uv_stderr = evidence / "uv/stderr.log"
        retry_events = [
            line
            for line in (
                uv_stderr.read_text(errors="replace") if uv_stderr.is_file() else ""
            ).splitlines()
            if any(
                message in line
                for message in ["could retry!", "max_retries_per_request hit"]
            )
        ]
        result["uv"]["protocol_retry_events"] = retry_events
        if retry_events:
            uv_errors.append("reqwest protocol replay was observed")
        if any(
            identity.get("final_path") != urlsplit(ARCHIVE_URL).path
            for identity in traces["response_identities"]
        ):
            uv_errors.append("Final response path differs")
        result["uv"]["control_errors"] = uv_errors
        result["curl"] = summarize_curl(curl_work, curl_child)
        write_json(evidence / "curl/transfer.json", result["curl"])
        if all(
            "finished_monotonic" in child and "spawn_returned_monotonic" in child
            for child in [uv_child, curl_child]
        ):
            result["observed_child_lifetime_overlap_seconds"] = max(
                0,
                min(uv_child["finished_monotonic"], curl_child["finished_monotonic"])
                - max(
                    uv_child["spawn_returned_monotonic"],
                    curl_child["spawn_returned_monotonic"],
                ),
            )
        result["overlap_limit"] = (
            "Child exit times and file growth are sampled about every 100 ms. The child lifetime overlap estimate may overstate actual overlap and does not establish overlapping requests or response bodies. Use the per-client progress observations and request traces for context."
        )
        result["status"] = (
            "timeout"
            if result["supervisor"]["timed_out"]
            else "success"
            if not uv_errors and result["curl"]["outcome"] == "success"
            else "unmatched_or_failed"
        )
    except (
        OSError,
        ValueError,
        RuntimeError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
    ) as error:
        result.update(
            status="harness_error", error_type=type(error).__name__, error=str(error)
        )
    result["finished_at"] = common.timestamp()
    write_json(evidence / "result.json", result)
    return result


def supervisor_check(output, workspace):
    work = workspace / "paired-supervisor-check"
    evidence = output / "paired-supervisor-check"
    work.mkdir()
    evidence.mkdir()
    script = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); time.sleep(30)"
    configuration = {
        "result_path": str(work / "worker.json"),
        "launch_order": ["uv", "curl"],
        "clients": {},
    }
    for client in ["uv", "curl"]:
        directory = work / client
        directory.mkdir()
        body = directory / "body"
        body.mkdir()
        configuration["clients"][client] = {
            "arguments": [sys.executable, "-c", script],
            "cwd": str(directory),
            "stdout": str(directory / "stdout.log"),
            "stderr": str(directory / "stderr.log"),
            "body_directory": str(body),
        }
    write_json(work / "configuration.json", configuration)
    result = run_process_tree(
        [
            sys.executable,
            str(Path(__file__).resolve()),
            "pair-worker",
            str(work / "configuration.json"),
        ],
        dict(os.environ),
        work,
        work / "stdout.log",
        work / "stderr.log",
        3,
    )
    result["worker"] = (
        json.loads((work / "worker.json").read_text())
        if (work / "worker.json").is_file()
        else None
    )
    write_json(evidence / "result.json", result)
    if (
        not result["timed_out"]
        or result["job_processes"] < 5
        or not result["tree_exit_confirmed"]
        or not result["worker"]
        or len(result["worker"]["clients"]) != 2
    ):
        raise RuntimeError("The paired process-tree deadline check failed")
    return result


def capture(args):
    if os.name != "nt":
        raise ValueError("The live reference capture requires Windows")
    repository = HERE.parent
    output, workspace = args.output.resolve(), args.workspace.resolve()
    output.mkdir(parents=True, exist_ok=True)
    workspace.mkdir(parents=True, exist_ok=False)
    if (output / "manifest.json").exists():
        raise ValueError("A capture manifest already exists")
    manifest = {
        "status": "preparing",
        "base_revision": BASE_REVISION,
        "expected_revision": args.expected_revision,
        "runner_index": args.runner_index,
        "hostname": socket.gethostname(),
        "platform": platform.platform(),
        "run_id": os.environ.get("GITHUB_RUN_ID"),
        "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
        "started_at": common.timestamp(),
        "pairs": [],
        "limits": [
            "The bound is 16 supervised logical transfers across both runners. Application retries are disabled, but reqwest and curl retain pre-response replay policies, so wire attempts can exceed 16. An observed replay makes a transfer unmatched. The absence of reqwest retry traces or additional curl connections does not prove a single wire attempt.",
            "Both clients request the same archive, address and HTTP/2 protocol. TLS fingerprints, HTTP/2 settings, flow control, certificate bundles and library implementations differ.",
            "The User-Agent, Accept and Accept-Encoding values are matched through a local HTTP/1 request capture. HTTP/2 header encoding and ordering remain client specific.",
            "uv retains its 30-second per-read idle timeout; curl uses its 120-second total timeout. The outer job enforces a 120-second deadline for each pair. These timeout policies are not equivalent.",
            "uv copy timing starts at its body-to-file copy. curl time_total minus time_starttransfer includes the remainder of response headers and is only an approximate body interval.",
            "Child lifetime and file growth are sampled every 100 ms. Process overlap does not establish exact request or body overlap; preserve the raw observations and CF-Ray correlation.",
            "Paired transfers can contend with each other and alter remote cache state. Public IP addresses do not identify a fixed physical edge. Four pairs per runner do not estimate a production failure rate.",
            "No application-level repeat or retry, resume, range request, cache-busting query or production configuration change is part of this capture.",
        ],
        "pair_count": 4,
        "planned_logical_transfers": 8,
        "application_retry_count": 0,
        "wire_attempt_limit": None,
        "sequence": SEQUENCES[args.runner_index],
        "system_proxy_observation": common.system_proxy_observation(),
        "proxy_environment_nonempty": {
            name: bool(os.environ.get(name)) for name in PROXY_NAMES
        },
    }
    write_json(output / "manifest.json", manifest)
    try:
        revision = common.git(repository, "rev-parse", "HEAD")
        tracked = common.git(
            repository, "status", "--porcelain", "--untracked-files=no"
        )
        preparation = json.loads((output / "checkout.json").read_text())
        if (
            revision != args.expected_revision
            or tracked
            or preparation["expected_revision"] != revision
            or preparation["after_reset"]
            != {"revision": revision, "tracked_status": ""}
        ):
            raise ValueError(
                "The pinned checkout was not clean before and after building"
            )
        uv, curl = args.uv.resolve(), args.curl.resolve()
        provenance = json.loads((curl.parent / "provenance.json").read_text())
        if (
            provenance["status"] != "verified_package"
            or provenance["archive_sha256"] != CURL_SHA256
            or common.digest(curl) != provenance["files"]["curl.exe"]["sha256"]
            or common.digest(curl.parent / "curl-ca-bundle.crt")
            != provenance["files"]["curl-ca-bundle.crt"]["sha256"]
        ):
            raise ValueError("The curl package provenance differs")
        manifest.update(
            revision=revision,
            tracked_status=tracked,
            source_sha256={
                name: common.digest(repository / name) for name in SOURCE_FILES
            },
            curl_provenance=provenance,
            uv_sha256=common.digest(uv),
            uv_version=subprocess.check_output(
                [uv, "--version"], text=True, timeout=10
            ).strip(),
            curl_version=subprocess.check_output(
                [curl, "--disable", "--version"], text=True, timeout=10
            ).strip(),
        )
        if (
            not manifest["curl_version"].startswith(f"curl {CURL_VERSION} ")
            or "HTTP2" not in manifest["curl_version"]
            or "nghttp2/" not in manifest["curl_version"]
        ):
            raise ValueError(
                "The pinned curl build lacks the expected version or HTTP/2 support"
            )
        proxy = manifest["system_proxy_observation"]
        if (
            proxy["proxy_enabled"]
            or proxy["proxy_server_nonempty"]
            or proxy["read_errors"]
        ):
            raise ValueError(
                "A system proxy is configured or unverified; client proxy parity must be reviewed before live requests"
            )
        metadata = json.loads(
            (repository / "crates/uv-python-managed/download-metadata.json").read_text()
        )[common.DOWNLOAD_KEY]
        if (
            metadata["build"] != common.BUILD
            or metadata["sha256"] != common.EXPECTED_ARCHIVE_SHA256
        ):
            raise ValueError("Pinned archive metadata differs")
        filename = urlsplit(metadata["url"]).path.rsplit("/", 1)[1].replace("%2B", "-")
        archive_name = f"{common.EXPECTED_ARCHIVE_SHA256[:9]}-{filename}"
        manifest["supervisor_check"] = supervisor_check(output, workspace)
        manifest["offline_check"] = common.offline_check(uv, output, workspace, "B")
        if manifest["offline_check"]["outcome"] != "passed":
            raise ValueError("The uv offline CLI preflight failed")
        manifest["tls_preflights"] = tls_preflights(
            uv, curl, output / "tls-preflights", workspace / "tls-preflights"
        )
        manifest["status"] = "running"
        write_json(output / "manifest.json", manifest)
        for index, first in enumerate(SEQUENCES[args.runner_index], 1):
            result = run_pair(
                uv,
                curl,
                curl.parent / "curl-ca-bundle.crt",
                manifest["tls_preflights"]["matched_request_headers"],
                output,
                workspace,
                index,
                first,
                archive_name,
            )
            manifest["pairs"].append(result)
            write_json(output / "manifest.json", manifest)
            if not result.get("supervisor", {}).get("tree_exit_confirmed"):
                raise RuntimeError(
                    "Pair tree exit is unconfirmed; remaining pairs were not started"
                )
        manifest["status"] = "completed"
    except (
        OSError,
        ValueError,
        RuntimeError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
    ) as error:
        manifest.update(
            status="harness_error", error_type=type(error).__name__, error=str(error)
        )
    manifest["finished_at"] = common.timestamp()
    write_json(output / "manifest.json", manifest)
    return int(
        manifest["status"] != "completed"
        or any(pair["status"] != "success" for pair in manifest["pairs"])
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    prepare = commands.add_parser("prepare-curl")
    prepare.add_argument("--directory", type=Path, required=True)
    prepare.add_argument("--archive", type=Path)
    prepare.add_argument("--evidence", type=Path)
    worker = commands.add_parser("pair-worker")
    worker.add_argument("configuration", type=Path)
    preflight = commands.add_parser("tls-preflights")
    preflight.add_argument("--uv", type=Path, required=True)
    preflight.add_argument("--curl", type=Path, required=True)
    preflight.add_argument("--output", type=Path, required=True)
    preflight.add_argument("--workspace", type=Path, required=True)
    run = commands.add_parser("capture")
    run.add_argument("--uv", type=Path, required=True)
    run.add_argument("--curl", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--workspace", type=Path, required=True)
    run.add_argument("--runner-index", type=int, choices=[1, 2], required=True)
    run.add_argument("--expected-revision", required=True)
    args = parser.parse_args()
    if args.action == "prepare-curl":
        try:
            prepare_curl(
                args.directory.resolve(),
                args.archive.resolve() if args.archive else None,
            )
        finally:
            provenance = args.directory / "provenance.json"
            if args.evidence and provenance.is_file():
                shutil.copyfile(provenance, args.evidence)
        return 0
    if args.action == "pair-worker":
        return pair_worker(args.configuration.resolve())
    if args.action == "tls-preflights":
        tls_preflights(
            args.uv.resolve(),
            args.curl.resolve(),
            args.output.resolve(),
            args.workspace.resolve(),
        )
        return 0
    if not re.fullmatch(r"[0-9a-f]{40}", args.expected_revision):
        parser.error("The expected revision must be a full commit SHA")
    return capture(args)


if __name__ == "__main__":
    raise SystemExit(main())
