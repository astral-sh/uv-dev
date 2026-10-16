"""Capture two bounded HTTP/1.1 and HTTP/2 `curl` pairs with receiver metadata."""

import argparse
import ctypes
import hashlib
import importlib.util
import ipaddress
import json
import os
import platform
import queue
import secrets
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

from python_reference_tls import BODY, Server
from python_transport_fixture import StreamingServer
from python_transport_stats import StatsError, WindowsStats, abi_layout
from python_transport_trace import TraceDrain
from windows_process_job import run_process_tree

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "reference_capture", HERE / "python-reference-capture.py"
)
assert SPEC is not None and SPEC.loader is not None
reference = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reference)
common = reference.common
PROTOCOL = json.loads((HERE / "python-transport-protocol.json").read_text())
SOURCE_FILES = [
    ".github/workflows/ci.yml",
    ".github/workflows/test.yml",
    "scripts/python-transport-capture.py",
    "scripts/python-transport-protocol.json",
    "scripts/python_transport_stats.py",
    "scripts/python_transport_trace.py",
    "scripts/python_transport_fixture.py",
    "scripts/python-reference-capture.py",
    "scripts/python_reference_tls.py",
    "scripts/python-address-capture.py",
    "scripts/windows_process_job.py",
    "scripts/python-reference-certificates/certificate.pem",
    "scripts/python-reference-certificates/private-key.pem",
]
NUMERIC_FIELDS = [
    "http_code",
    "remote_port",
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


def write_json(path, value):
    common.write_json(path, value)


def failure_metadata(error, operation):
    """Retain operation and numeric failure codes without exception text."""
    result = {"error_type": type(error).__name__, "error_operation": operation}
    if isinstance(error, StatsError):
        result.update(error_operation=error.operation, error_code=error.code)
    for name in ["errno", "winerror", "returncode"]:
        value = getattr(error, name, None)
        if isinstance(value, int):
            result[f"error_{name}"] = value
    return result


def arguments(curl, ca, url, peer, protocol, body, size):
    values = reference.curl_arguments(
        curl, ca, url, f"{peer}:0", body, Path(os.devnull), PROTOCOL["request_headers"]
    )
    index = values.index("--dump-header")
    del values[index : index + 2]
    if protocol == "h1":
        values[values.index("--http2")] = "--http1.1"
    elif protocol != "h2":
        raise ValueError("The protocol arm is invalid")
    values[-1:-1] = ["--verbose", "--trace-config", "http/2,ids,time"]
    if size:
        values[-1:-1] = ["--max-filesize", str(size)]
    return values


class StdoutDrain:
    def __init__(self, stream, body_bytes=0, pause_seconds=0):
        self.stream = stream
        self.body_bytes = body_bytes
        self.pause_seconds = pause_seconds
        self.buffer = bytearray()
        self.digest = hashlib.sha256()
        self.record = {
            "body_bytes": 0,
            "json_bytes": 0,
            "overflow": False,
            "finished": False,
        }
        self.thread = threading.Thread(target=self.run, daemon=True)

    def run(self):
        paused = False
        try:
            while chunk := self.stream.read1(65536):
                remaining = self.body_bytes - self.record["body_bytes"]
                count = min(len(chunk), remaining)
                if count:
                    self.digest.update(chunk[:count])
                    self.record["body_bytes"] += count
                    if self.pause_seconds and not paused:
                        self.record["pause_started_monotonic"] = time.monotonic()
                        time.sleep(self.pause_seconds)
                        self.record["pause_finished_monotonic"] = time.monotonic()
                        paused = True
                suffix = chunk[count:]
                self.record["json_bytes"] += len(suffix)
                if len(self.buffer) + len(suffix) <= 65536:
                    self.buffer.extend(suffix)
                else:
                    self.record["overflow"] = True
        except OSError as error:
            self.record.update(failure_metadata(error, "curl_stdout_read"))
        finally:
            self.record.update(finished=True, body_sha256=self.digest.hexdigest())

    def transfer(self, expected_url):
        if not self.record["finished"] or self.record["overflow"]:
            return {"completion_metadata_available": False}
        try:
            value = json.loads(self.buffer)
            result = {
                name: value.get(name)
                for name in NUMERIC_FIELDS
                if isinstance(value.get(name), (int, float))
            }
            result["http_version"] = (
                str(value.get("http_version"))
                if str(value.get("http_version")) in ("1.1", "2")
                else None
            )
            try:
                result["remote_ip"] = str(ipaddress.IPv4Address(value.get("remote_ip")))
            except ipaddress.AddressValueError:
                result["remote_ip"] = None
            result["url_matches_expected"] = value.get("url_effective") == expected_url
            result["completion_metadata_available"] = True
            return result
        except (ValueError, TypeError, KeyError):
            return {"completion_metadata_available": False}


class StatsSampler:
    def __init__(self, process, peer, port):
        self.process, self.peer, self.port = process, peer, port
        self.events = queue.Queue(maxsize=64)
        self.record = {
            "samples": 0,
            "encoded_bytes": 0,
            "dropped_samples": 0,
            "discovery_calls": 0,
            "maximum_sample_seconds": 0,
            "maximum_schedule_gap_seconds": 0,
            "valid_observations": 0,
            "invalid_categories": 0,
            "finished": False,
            "incomplete_samples": 0,
        }
        self.thread = threading.Thread(target=self.run, daemon=True)

    def run(self):
        started = time.monotonic()
        cpu_started = time.thread_time()
        previous = started
        try:
            reader = WindowsStats(self.process, self.peer, self.port)
            self.record["process_creation_time"] = reader.creation_time
            while self.process.poll() is None:
                before = time.monotonic()
                self.record["maximum_schedule_gap_seconds"] = max(
                    self.record["maximum_schedule_gap_seconds"], before - previous
                )
                previous = before
                sample = reader.sample()
                self.record["incomplete_samples"] += int(sample["status"] != "live")
                self.record["discovery_calls"] += 1
                self.record["maximum_sample_seconds"] = max(
                    self.record["maximum_sample_seconds"], time.monotonic() - before
                )
                if sample["observations"]:
                    self.record.setdefault("first_observation_monotonic", before)
                for observation in sample["observations"]:
                    valid = (
                        all(
                            category.get("status") == 0
                            and category.get("enabled")
                            and "values" in category
                            for category in observation["categories"].values()
                        )
                        and len(observation["categories"]) == 3
                        and sample["status"] == "live"
                        and not observation.get("collection_continuity_changed")
                    )
                    self.record["valid_observations"] += int(valid)
                    self.record["invalid_categories"] += sum(
                        "values" not in category
                        for category in observation["categories"].values()
                    )
                if (
                    sample["observations"]
                    or sample["status"] != "live"
                    or sample.get("disappeared_generations")
                ):
                    encoded = (
                        json.dumps(sample, separators=(",", ":")) + "\n"
                    ).encode()
                    if (
                        self.record["samples"] >= PROTOCOL["stats_samples_limit"]
                        or self.record["encoded_bytes"] + len(encoded)
                        > PROTOCOL["stats_bytes_limit"]
                    ):
                        self.record["dropped_samples"] += 1
                        self.record["cap_reached"] = True
                        break
                    try:
                        self.events.put_nowait(encoded)
                    except queue.Full:
                        self.record["dropped_samples"] += 1
                    else:
                        self.record["samples"] += 1
                        self.record["encoded_bytes"] += len(encoded)
                if sample["status"] != "live":
                    self.record["terminal_sample_status"] = sample["status"]
                    break
                if (
                    not self.record["valid_observations"]
                    and time.monotonic() - started >= 10
                ):
                    self.record["attachment_deadline_reached"] = True
                    break
                interval = 0.1 if self.record["valid_observations"] else 0.01
                time.sleep(max(0, interval - (time.monotonic() - before)))
        except (OSError, ValueError, RuntimeError) as error:
            self.record.update(failure_metadata(error, "receiver_sampling"))
        finally:
            self.record.update(
                finished=True, thread_cpu_seconds=time.thread_time() - cpu_started
            )

    def take(self):
        lines = []
        while True:
            try:
                lines.append(self.events.get_nowait())
            except queue.Empty:
                return lines


def run_transfer(configuration, directory, *, canaries=None):
    try:
        return transfer(configuration, directory, canaries=canaries)
    except (
        OSError,
        ValueError,
        RuntimeError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
    ) as error:
        path = directory / "result.json"
        result = json.loads(path.read_text()) if path.is_file() else {}
        result.update(
            status="error",
            finished_at=common.timestamp(),
            **failure_metadata(error, result.get("stage", "transfer_setup")),
        )
        write_json(path, result)
        raise


def transfer(configuration, directory, *, canaries=None):
    directory.mkdir(parents=True, exist_ok=True)
    result_path = directory / "result.json"
    result = {
        "status": "starting",
        "started_at": common.timestamp(),
        "started_monotonic": time.monotonic(),
        "progress": [],
        "protocol_arm": configuration["protocol"],
        "stage": "curl_environment",
    }
    write_json(result_path, result)
    environment = dict(os.environ)
    environment.update(configuration.get("environment", {}))
    if environment.get("SSLKEYLOGFILE"):
        raise ValueError("TLS key logging must be absent for this capture")
    cpu_started = time.process_time()
    result["stage"] = "curl_spawn"
    write_json(result_path, result)
    process = subprocess.Popen(
        configuration["arguments"],
        cwd=directory,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        close_fds=True,
    )
    result.update(
        pid=process.pid,
        spawn_returned_monotonic=time.monotonic(),
        status="running",
        stage="curl_stream_capture",
    )
    trace = TraceDrain(process.stderr, canaries)
    stdout = StdoutDrain(
        process.stdout,
        configuration.get("stdout_body_bytes", 0),
        configuration.get("stdout_pause_seconds", 0),
    )
    stats = (
        StatsSampler(process, configuration["peer"], configuration["port"])
        if configuration.get("stats")
        else None
    )
    trace.start()
    stdout.thread.start()
    if stats:
        stats.thread.start()
    body = Path(configuration["body_path"]) if configuration.get("body_path") else None
    with (
        (directory / "h2-events.jsonl").open("wb") as trace_output,
        (directory / "receiver-samples.jsonl").open("wb") as stats_output,
    ):
        while True:
            trace_output.writelines(trace.take())
            if stats:
                stats_output.writelines(stats.take())
            trace_output.flush()
            stats_output.flush()
            now = time.monotonic()
            count = (
                body.stat().st_size
                if body and body.is_file()
                else stdout.record["body_bytes"]
            )
            if not result["progress"] or result["progress"][-1]["bytes"] != count:
                result["progress"].append({"observed_monotonic": now, "bytes": count})
            result.update(trace=trace.snapshot(), stdout=dict(stdout.record))
            if stats:
                result["receiver"] = dict(stats.record)
            write_json(result_path, result)
            if process.poll() is not None:
                break
            time.sleep(0.05)
        trace.thread.join(timeout=1)
        stdout.thread.join(timeout=1)
        if stats:
            stats.thread.join(timeout=1)
        trace_output.writelines(trace.take())
        if stats:
            stats_output.writelines(stats.take())
    result.update(
        status="completed",
        stage="curl_completion",
        exit_code=process.returncode,
        finished_monotonic=time.monotonic(),
        finished_at=common.timestamp(),
        trace=trace.snapshot(),
        stdout=dict(stdout.record),
        worker_cpu_seconds=time.process_time() - cpu_started,
        transfer=stdout.transfer(configuration["url"]),
    )
    if stats:
        result["receiver"] = dict(stats.record)
    if body and body.is_file():
        result.update(body_bytes=body.stat().st_size, body_sha256=common.digest(body))
    else:
        result.update(
            body_bytes=stdout.record["body_bytes"],
            body_sha256=stdout.record.get("body_sha256"),
        )
    write_json(result_path, result)
    return result


def local_configuration(
    curl,
    ca,
    server,
    protocol,
    *,
    body_bytes=0,
    stats=False,
    hostname=reference.HOST,
    environment=None,
    pause=0,
):
    url = f"https://{hostname}:{server.port}/fixture"
    return {
        "arguments": arguments(curl, ca, url, "127.0.0.1", protocol, "-", body_bytes),
        "url": url,
        "peer": "127.0.0.1",
        "port": server.port,
        "protocol": protocol,
        "stats": stats,
        "stdout_body_bytes": body_bytes,
        "stdout_pause_seconds": pause,
        "environment": environment or reference.local_environment(),
    }


def categories_valid(result):
    receiver = result.get("receiver", {})
    return (
        receiver.get("valid_observations", 0) > 0
        and receiver.get("invalid_categories", 0) == 0
        and receiver.get("dropped_samples", 0) == 0
        and receiver.get("finished") is True
        and "error_type" not in receiver
    )


def trace_valid(result):
    trace = result.get("trace", {})
    return (
        trace.get("finished") is True
        and trace.get("dropped_events") == 0
        and trace.get("selected_parse_errors") == 0
        and trace.get("oversize_lines") == 0
        and "reader_error_type" not in trace
    )


def read_events(path):
    if not path.is_file():
        return []
    values = []
    for line in path.read_bytes().splitlines():
        try:
            values.append(json.loads(line))
        except ValueError:
            break
    return values


def raw_receiver(port, noise_port):
    with (
        socket.create_connection(("127.0.0.1", port), timeout=10) as connection,
        socket.create_connection(("127.0.0.1", noise_port), timeout=10) as noise,
    ):
        connection.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 65536)
        noise.sendall(b"local exclusion control")
        print(
            json.dumps(
                {
                    "phase": 1,
                    "monotonic": time.monotonic(),
                    "local_port": connection.getsockname()[1],
                }
            ),
            flush=True,
        )
        time.sleep(2)
        print(json.dumps({"phase": 2, "monotonic": time.monotonic()}), flush=True)
        count = 0
        while count < 16 * 1024 * 1024:
            chunk = connection.recv(min(65536, 16 * 1024 * 1024 - count))
            if not chunk:
                break
            count += len(chunk)
        print(
            json.dumps({"phase": 3, "monotonic": time.monotonic(), "bytes": count}),
            flush=True,
        )
        time.sleep(1)
        return int(count != 16 * 1024 * 1024)


def receiver_control(directory):
    directory.mkdir()
    listeners = []
    errors = []
    sender_endpoint = {}
    for _ in range(2):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(10)
        listeners.append(listener)
    ports = [listener.getsockname()[1] for listener in listeners]

    def send(listener, target):
        try:
            connection, _peer = listener.accept()
            with connection:
                connection.settimeout(10)
                if target:
                    sender_endpoint.update(
                        local_port=connection.getsockname()[1],
                        remote_port=connection.getpeername()[1],
                    )
                    for _ in range(256):
                        connection.sendall(b"r" * 65536)
                    connection.recv(1)
                else:
                    connection.recv(128)
        except OSError as error:
            errors.append(type(error).__name__)
        finally:
            listener.close()

    threads = [
        threading.Thread(target=send, args=(listener, index == 0), daemon=True)
        for index, listener in enumerate(listeners)
    ]
    for thread in threads:
        thread.start()
    process = subprocess.Popen(
        [
            sys.executable,
            str(Path(__file__).resolve()),
            "raw-receiver",
            str(ports[0]),
            str(ports[1]),
        ],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    phases = []

    def receive_phases():
        while line := process.stdout.readline(512):
            try:
                value = json.loads(line)
                if (
                    value.get("phase") in (1, 2, 3)
                    and isinstance(value.get("monotonic"), (int, float))
                    and len(phases) < 3
                ):
                    phases.append(
                        {
                            key: number
                            for key, number in value.items()
                            if key in ("phase", "monotonic", "local_port", "bytes")
                            and isinstance(number, (int, float))
                        }
                    )
            except (ValueError, TypeError):
                pass

    phase_reader = threading.Thread(target=receive_phases, daemon=True)
    phase_reader.start()
    sampler = StatsSampler(process, "127.0.0.1", ports[0])
    sampler.thread.start()
    with (directory / "receiver-samples.jsonl").open("wb") as output:
        while process.poll() is None:
            output.writelines(sampler.take())
            output.flush()
            time.sleep(0.05)
        sampler.thread.join(timeout=1)
        output.writelines(sampler.take())
    for thread in threads:
        thread.join(timeout=1)
    phase_reader.join(timeout=1)
    samples = read_events(directory / "receiver-samples.jsonl")
    observations = [
        observation
        for sample in samples
        if sample["status"] == "live"
        for observation in sample["observations"]
    ]
    receivers = [
        item["categories"]["receiver"].get("values", {}) for item in observations
    ]
    peak_queue = max((item.get("CurAppRQueue", 0) for item in receivers), default=0)
    windows = [item["CurRwinSent"] for item in receivers if "CurRwinSent" in item]
    phase_times = {phase["phase"]: phase["monotonic"] for phase in phases}
    paused = [
        observation["categories"]["receiver"].get("values", {})
        for sample in samples
        if sample["status"] == "live"
        and phase_times.get(1, float("inf")) <= sample["started_monotonic"]
        and sample["finished_monotonic"] < phase_times.get(2, 0)
        for observation in sample["observations"]
    ]
    recovered = [
        observation["categories"]["receiver"].get("values", {})
        for sample in samples
        if sample["status"] == "live"
        and sample["started_monotonic"] > phase_times.get(3, float("inf"))
        for observation in sample["observations"]
    ]
    paused_peak = max((value.get("CurAppRQueue", 0) for value in paused), default=0)
    paused_minimum_window = min(
        (value.get("CurRwinSent", 0) for value in paused), default=0
    )
    recovered_open = any(
        value.get("CurAppRQueue") == 0
        and value.get("CurRwinSent", 0) > paused_minimum_window
        for value in recovered
    )
    record = {
        "receiver": sampler.record,
        "exit_code": process.returncode,
        "server_error_types": errors,
        "peak_unread_queue": peak_queue,
        "minimum_advertised_window": min(windows) if windows else None,
        "maximum_advertised_window": max(windows) if windows else None,
        "phases": phases,
        "sender_endpoint": sender_endpoint,
        "paused_unread_queue_peak": paused_peak,
        "recovered_empty_queue_open_window": recovered_open,
        "reverse_sender_tuple_matches": bool(observations)
        and all(
            item["connection"]["local_port"] == sender_endpoint.get("remote_port")
            and item["connection"]["remote_port"] == sender_endpoint.get("local_port")
            for item in observations
        ),
        "all_rows_match_owned_receiver": bool(observations)
        and all(
            item["pid"] == process.pid
            and item["connection"]["remote_port"] == ports[0]
            and item["connection"]["remote_address"] == "127.0.0.1"
            for item in observations
        ),
        "unrelated_port_excluded": all(
            item["connection"]["remote_port"] != ports[1] for item in observations
        ),
    }
    record["passed"] = (
        process.returncode == 0
        and not errors
        and categories_valid(record)
        and peak_queue > 0
        and bool(windows)
        and min(windows) < max(windows)
        and record["all_rows_match_owned_receiver"]
        and record["unrelated_port_excluded"]
        and record["reverse_sender_tuple_matches"]
        and paused_peak > 0
        and recovered_open
    )
    write_json(directory / "result.json", record)
    if not record["passed"]:
        raise ValueError("The receiver attribution/backpressure control failed")
    return record


CONTROL_DEADLINES = {
    "abi-elevation": 5,
    "receiver-control": 15,
    "http1-headers-body": 5,
    "http2-body": 5,
    "wrong-hostname": 5,
    "untrusted-certificate": 5,
    "refused-stream": 15,
    "configured-proxy": 5,
    "numeric-h2": 15,
    "numeric-h2-baseline": 15,
}


def effective_elevation():
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    security = ctypes.WinDLL("advapi32", use_last_error=True)
    security.OpenProcessToken.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    security.OpenProcessToken.restype = ctypes.c_int32
    security.GetTokenInformation.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int32,
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.POINTER(ctypes.c_uint32),
    ]
    security.GetTokenInformation.restype = ctypes.c_int32
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel.CloseHandle.restype = ctypes.c_int32
    token = ctypes.c_void_p()
    if not security.OpenProcessToken(ctypes.c_void_p(-1), 8, ctypes.byref(token)):
        raise StatsError("OpenProcessToken", ctypes.get_last_error())
    try:
        elevated = ctypes.c_uint32()
        returned = ctypes.c_uint32()
        if not security.GetTokenInformation(
            token,
            20,
            ctypes.byref(elevated),
            ctypes.sizeof(elevated),
            ctypes.byref(returned),
        ):
            raise StatsError(
                "GetTokenInformation.TokenElevation", ctypes.get_last_error()
            )
        return {"elevated": bool(elevated.value), "returned_bytes": returned.value}
    finally:
        kernel.CloseHandle(token)


def run_bounded(arguments, cwd, deadline):
    if os.name == "nt":
        return run_process_tree(
            arguments,
            dict(os.environ),
            cwd,
            Path(os.devnull),
            Path(os.devnull),
            deadline,
        )
    started = time.monotonic()
    process = subprocess.Popen(
        arguments,
        cwd=cwd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    timed_out = False
    try:
        process.wait(timeout=deadline)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)
    return {
        "exit_code": process.returncode,
        "timed_out": timed_out,
        "elapsed_seconds": time.monotonic() - started,
        "platform_limit": "Linux fixture process group; Windows Job Object was not executed",
    }


def orchestrate_preflights(curl, directory, portable):
    directory.mkdir(parents=True, exist_ok=True)
    record = {
        "status": "running",
        "portable_only": portable,
        "checks": [],
        "platform": platform.platform(),
        "curl_version": subprocess.check_output(
            [curl, "--disable", "--version"], text=True, timeout=5
        ).strip(),
    }
    write_json(directory / "result.json", record)
    for case, deadline in CONTROL_DEADLINES.items():
        if portable and case == "receiver-control":
            continue
        child_directory = directory / case
        child_directory.mkdir()
        command = [
            sys.executable,
            str(Path(__file__).resolve()),
            "preflight-worker",
            "--curl",
            str(curl),
            "--output",
            str(child_directory),
            "--case",
            case,
        ]
        if portable:
            command.append("--portable")
        supervisor = run_bounded(command, child_directory, deadline)
        path = child_directory / "result.json"
        child = (
            json.loads(path.read_text()) if path.is_file() else {"status": "missing"}
        )
        passed = (
            supervisor["exit_code"] == 0
            and not supervisor["timed_out"]
            and child["status"] == "passed"
        )
        if not portable:
            passed = passed and supervisor.get("tree_exit_confirmed") is True
        record["checks"].append(
            {
                "name": case,
                "deadline_seconds": deadline,
                "supervisor": supervisor,
                "passed": passed,
                "result": child,
            }
        )
        write_json(directory / "result.json", record)
        if not passed:
            record.update(status="failed", failed_gate=case)
            write_json(directory / "result.json", record)
            return 1
    record["status"] = "passed"
    if portable:
        record["native_gates_unexecuted"] = [
            "effective Windows token",
            "Windows EStats",
            "Windows Job Object",
            "pinned Windows curl executable",
        ]
    write_json(directory / "result.json", record)
    return 0


def preflight_worker(curl, directory, portable=False, case=None):
    if case is None:
        return orchestrate_preflights(curl, directory, portable)
    directory.mkdir(parents=True, exist_ok=True)
    certificate = HERE / "python-reference-certificates/certificate.pem"
    private_key = HERE / "python-reference-certificates/private-key.pem"
    record = {
        "status": "running",
        "platform": platform.platform(),
        "portable_only": portable,
        "checks": [],
    }
    write_json(directory / "result.json", record)

    def retain(name, value, passed):
        record["checks"].append({"name": name, "passed": passed, "result": value})
        write_json(directory / "result.json", record)
        if not passed:
            raise ValueError(f"The local control failed: {name}")

    try:
        layout = abi_layout()
        expected = {
            "TcpRow": 20,
            "OwnedTcpRow": 24,
            "Collection": 1,
            "Receiver": 72,
            "Data": 96,
            "NetworkPath": 160,
        }
        retain(
            "x64-abi-layout",
            layout,
            all(layout[name]["size"] == size for name, size in expected.items()),
        )
        if case == "abi-elevation" and not portable:
            elevation = effective_elevation()
            retain("effective-token-elevation", elevation, elevation["elevated"])
        if case == "receiver-control":
            retain(
                "receiver-backpressure-attribution",
                receiver_control(directory / "receiver-control"),
                True,
            )
        for name, protocol, hostname, trusted in [
            ("http1-headers-body", "h1", reference.HOST, True),
            ("http2-body", "h2", reference.HOST, True),
            ("wrong-hostname", "h2", "wrong-host.invalid", True),
            ("untrusted-certificate", "h2", reference.HOST, False),
        ]:
            if name != case:
                continue
            server = Server(
                certificate, private_key, "h2" if protocol == "h2" else "http/1.1"
            )
            ca = certificate if trusted else curl.parent / "curl-ca-bundle.crt"
            if portable and not ca.is_file():
                ca = Path("/etc/ssl/certs/ca-certificates.crt")
            success = trusted and hostname == reference.HOST
            configuration = local_configuration(
                curl,
                ca,
                server,
                protocol,
                body_bytes=len(BODY) if success else 0,
                hostname=hostname,
            )
            result = run_transfer(configuration, directory / name)
            observed = server.finish()
            result["server"] = observed
            if success:
                passed = (
                    result["exit_code"] == 0
                    and result["body_sha256"] == hashlib.sha256(BODY).hexdigest()
                    and observed.get("sni") == [reference.HOST]
                    and observed.get("alpn")
                    == ("h2" if protocol == "h2" else "http/1.1")
                    and result["transfer"].get("http_version")
                    == ("2" if protocol == "h2" else "1.1")
                    and observed["requests"] == 1
                )
                if protocol == "h1":
                    passed = passed and all(
                        observed["request_headers"].get(key) == value
                        for key, value in PROTOCOL["request_headers"].items()
                    )
            else:
                passed = result["exit_code"] == 60 and observed["requests"] == 0
            retain(name, result, passed)
        if case == "refused-stream":
            refused = Server(certificate, private_key, "h2", refuse_stream=True)
            result = run_transfer(
                local_configuration(curl, certificate, refused, "h2"),
                directory / "refused-stream",
            )
            result["server"] = refused.finish()
            retain(
                "default-refused-stream-policy",
                result,
                result["exit_code"] != 0
                and (
                    portable
                    or (
                        result["server"]["requests"] == 6
                        and result["transfer"].get("num_connects") == 6
                    )
                ),
            )
        if case == "configured-proxy":
            proxy = Server(certificate, private_key, proxy=True)
            environment = reference.local_environment()
            environment.update(
                HTTPS_PROXY=f"http://127.0.0.1:{proxy.port}",
                https_proxy=f"http://127.0.0.1:{proxy.port}",
                NO_PROXY="",
                no_proxy="",
            )
            target = type("ProxyTarget", (), {"port": 9})()
            result = run_transfer(
                local_configuration(
                    curl, certificate, target, "h2", environment=environment
                ),
                directory / "configured-proxy",
            )
            result["server"] = proxy.finish()
            retain(
                "configured-proxy",
                result,
                result["exit_code"] != 0
                and result["server"].get("connect_authority") == f"{reference.HOST}:9",
            )
        if case in ("numeric-h2", "numeric-h2-baseline"):
            canaries = {
                name: secrets.token_hex(16).encode()
                for name in ["body", "header", "trailer"]
            }
            stream = StreamingServer(
                certificate, private_key, "h2", 16 * 1024 * 1024, canaries
            )
            configuration = local_configuration(
                curl,
                certificate,
                stream,
                "h2",
                body_bytes=16 * 1024 * 1024,
                stats=not portable,
                pause=2,
            )
            if case == "numeric-h2-baseline":
                configuration["arguments"].remove("--verbose")
                index = configuration["arguments"].index("--trace-config")
                del configuration["arguments"][index : index + 2]
            result = run_transfer(
                configuration, directory / "numeric-h2", canaries=canaries
            )
            result["server"] = stream.finish()
            events = read_events(directory / "numeric-h2/h2-events.jsonl")
            kinds = sorted({event["event"] for event in events})
            exported = (
                directory / "numeric-h2/h2-events.jsonl"
            ).read_bytes() + json.dumps(result).encode()
            privacy = not any(
                value in exported for value in canaries.values()
            ) and not result["trace"]["canaries_seen"].get("body", False)
            challenge_seen = all(
                result["trace"]["canaries_seen"].get(name, False)
                for name in ["header", "trailer"]
            )
            result.update(
                event_kinds=kinds,
                exported_canaries_absent=privacy,
                header_trailer_challenges_seen=challenge_seen,
            )
            expected_body = hashlib.sha256(
                canaries["body"] + b"b" * (16 * 1024 * 1024 - len(canaries["body"]))
            ).hexdigest()
            passed = (
                result["exit_code"] == 0
                and result["body_bytes"] == 16 * 1024 * 1024
                and result["body_sha256"] == expected_body
                and result["transfer"].get("http_version") == "2"
                and privacy
                and (case == "numeric-h2-baseline" or challenge_seen)
                and (
                    portable
                    or (
                        categories_valid(result)
                        and (
                            case == "numeric-h2-baseline"
                            or ({1, 2, 5}.issubset(kinds) and trace_valid(result))
                        )
                    )
                )
            )
            retain("numeric-h2-privacy-positive-control", result, passed)
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
            status="failed", failed_gate=case, **failure_metadata(error, case)
        )
    write_json(directory / "result.json", record)
    return int(record["status"] != "passed")


def native_blocker(leaf=False):
    if os.name != "nt":
        raise ValueError("The native deadline control requires Windows")
    if not leaf:
        subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), "native-blocker", "--leaf"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    kernel = ctypes.WinDLL("kernel32")
    kernel.Sleep.argtypes = [ctypes.c_uint32]
    kernel.Sleep.restype = None
    kernel.Sleep(30000)


def pair_worker(configuration_path):
    configuration = json.loads(configuration_path.read_text())
    result_path = Path(configuration["result_path"])
    result = {"status": "starting", "workers": {}}
    processes = {}
    try:
        for arm in configuration["launch_order"]:
            directory = Path(configuration["directories"][arm])
            process = subprocess.Popen(
                [
                    sys.executable,
                    str(Path(__file__).resolve()),
                    "transfer-worker",
                    str(directory / "configuration.json"),
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            processes[arm] = process
            result["workers"][arm] = {
                "pid": process.pid,
                "launched_monotonic": time.monotonic(),
            }
            write_json(result_path, result)
        result["status"] = "running"
        while any(process.poll() is None for process in processes.values()):
            for arm, process in processes.items():
                if process.poll() is not None:
                    result["workers"][arm].setdefault("exit_code", process.returncode)
                    result["workers"][arm].setdefault(
                        "finished_monotonic", time.monotonic()
                    )
            write_json(result_path, result)
            time.sleep(0.05)
        result["status"] = "completed"
        for arm, process in processes.items():
            result["workers"][arm].update(
                exit_code=process.wait(), finished_monotonic=time.monotonic()
            )
    except (OSError, ValueError, RuntimeError, KeyError, TypeError) as error:
        result.update(status="error", **failure_metadata(error, "pair_worker"))
        write_json(result_path, result)
        # The outer job remains responsible for every child if a worker fails.
        for process in processes.values():
            process.wait()
    write_json(result_path, result)
    return int(result["status"] != "completed")


def classify(result, arm):
    transfer = result.get("transfer", {})
    reasons = []
    if not transfer.get("completion_metadata_available"):
        reasons.append("completion_identity_unavailable")
    else:
        expected = {
            "http_code": 200,
            "http_version": "1.1" if arm == "h1" else "2",
            "remote_ip": PROTOCOL["peer_address"],
            "remote_port": PROTOCOL["peer_port"],
            "num_connects": 1,
            "num_redirects": 0,
            "num_retries": 0,
            "ssl_verify_result": 0,
            "url_matches_expected": True,
        }
        reasons.extend(
            f"{name}_mismatch"
            for name, value in expected.items()
            if transfer.get(name) != value
        )
    if result.get("exit_code") != 0:
        reasons.append("child_incomplete_or_failed")
    if (
        result.get("body_bytes") != PROTOCOL["archive_size"]
        or result.get("body_sha256") != PROTOCOL["archive_sha256"]
    ):
        reasons.append("archive_incomplete_or_mismatched")
    if not categories_valid(result):
        reasons.append("receiver_telemetry_incomplete")
    if not trace_valid(result):
        reasons.append("diagnostic_telemetry_incomplete")
    if len(result.get("trace", {}).get("connection_ids", [])) > 1:
        reasons.append("multiple_diagnostic_connections")
    if arm == "h2" and result.get("trace", {}).get("accepted_events", 0) == 0:
        reasons.append("h2_events_missing")
    duration = None
    if all(
        isinstance(transfer.get(key), (int, float))
        for key in ["time_total", "time_starttransfer"]
    ):
        duration = transfer["time_total"] - transfer["time_starttransfer"]
    return {
        "status": "matched" if not reasons else "unmatched_or_incomplete",
        "reasons": reasons,
        "after_first_response_byte_seconds": duration,
        "slow_body_observed": duration is not None
        and duration >= PROTOCOL["slow_body_threshold_seconds"],
        "timing_limit": "The curl interval starts at first response byte and can include the remainder of response headers.",
    }


def run_pair(curl, evidence, workspace, index, first):
    directory = evidence / f"pair-{index:02d}"
    directory.mkdir()
    work = workspace / f"pair-{index:02d}"
    work.mkdir()
    order = [first, "h2" if first == "h1" else "h1"]
    configuration = {
        "launch_order": order,
        "directories": {},
        "result_path": str(directory / "workers.json"),
    }
    for arm in ["h1", "h2"]:
        child_directory = directory / arm
        child_directory.mkdir()
        body = work / f"{arm}.tar.gz"
        child = {
            "mode": "live",
            "protocol": arm,
            "peer": PROTOCOL["peer_address"],
            "port": PROTOCOL["peer_port"],
            "url": PROTOCOL["archive_url"],
            "stats": True,
            "body_path": str(body),
            "arguments": arguments(
                curl,
                curl.parent / "curl-ca-bundle.crt",
                PROTOCOL["archive_url"],
                PROTOCOL["peer_address"],
                arm,
                body,
                PROTOCOL["archive_size"],
            ),
            "output": str(child_directory),
        }
        write_json(child_directory / "configuration.json", child)
        configuration["directories"][arm] = str(child_directory)
    write_json(directory / "configuration.json", configuration)
    result = {
        "pair": index,
        "launch_order": order,
        "status": "running",
        "started_at": common.timestamp(),
    }
    write_json(directory / "result.json", result)
    result["supervisor"] = run_process_tree(
        [
            sys.executable,
            str(Path(__file__).resolve()),
            "pair-worker",
            str(directory / "configuration.json"),
        ],
        dict(os.environ),
        work,
        work / "stdout.log",
        work / "stderr.log",
        PROTOCOL["pair_deadline_seconds"],
    )
    result["arms"] = {}
    for arm in ["h1", "h2"]:
        path = directory / arm / "result.json"
        child = (
            json.loads(path.read_text()) if path.is_file() else {"status": "missing"}
        )
        body = work / f"{arm}.tar.gz"
        if body.is_file() and "body_bytes" not in child:
            child.update(
                body_bytes=body.stat().st_size, body_sha256=common.digest(body)
            )
        result["arms"][arm] = {"child": child, "classification": classify(child, arm)}
    children = [value["child"] for value in result["arms"].values()]
    if all(
        "finished_monotonic" in child and "spawn_returned_monotonic" in child
        for child in children
    ):
        result["observed_child_lifetime_overlap_seconds"] = max(
            0,
            min(child["finished_monotonic"] for child in children)
            - max(child["spawn_returned_monotonic"] for child in children),
        )
    result["status"] = "timeout" if result["supervisor"]["timed_out"] else "completed"
    result["finished_at"] = common.timestamp()
    write_json(directory / "result.json", result)
    return result


def capture(args):
    if os.name != "nt":
        raise ValueError("The live transport capture requires Windows")
    output, workspace, curl = (
        args.output.resolve(),
        args.workspace.resolve(),
        args.curl.resolve(),
    )
    output.mkdir(parents=True, exist_ok=True)
    workspace.mkdir(parents=True, exist_ok=False)
    if (output / "manifest.json").exists():
        raise ValueError("A capture manifest already exists")
    manifest = {
        "status": "preparing",
        "protocol": PROTOCOL,
        "runner_index": args.runner_index,
        "hostname": socket.gethostname(),
        "platform": platform.platform(),
        "run_id": os.environ.get("GITHUB_RUN_ID"),
        "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
        "started_at": common.timestamp(),
        "pairs": [],
        "system_proxy_observation": common.system_proxy_observation(),
        "proxy_environment_nonempty": {
            name: bool(os.environ.get(name)) for name in reference.PROXY_NAMES
        },
    }
    write_json(output / "manifest.json", manifest)
    try:
        stage = "checkout_identity"
        revision = common.git(HERE.parent, "rev-parse", "HEAD")
        tracked = common.git(
            HERE.parent, "status", "--porcelain", "--untracked-files=no"
        )
        preparation = json.loads((output / "checkout.json").read_text())
        manifest["checkout_identity"] = {
            "revision": revision,
            "tracked_clean": not tracked,
            "expected_revision_matches": revision == args.expected_revision,
            "reset_matches": preparation.get("after_reset")
            == {"revision": revision, "tracked_status": ""},
        }
        if (
            revision != args.expected_revision
            or tracked
            or preparation["after_reset"]
            != {"revision": revision, "tracked_status": ""}
        ):
            raise ValueError("The capture checkout differs from its pinned revision")
        stage = "curl_package_identity"
        provenance = json.loads((curl.parent / "provenance.json").read_text())
        if (
            provenance["status"] != "verified_package"
            or provenance["archive_sha256"] != PROTOCOL["curl_zip_sha256"]
            or common.digest(curl) != PROTOCOL["curl_exe_sha256"]
            or common.digest(curl.parent / "curl-ca-bundle.crt")
            != PROTOCOL["curl_ca_sha256"]
        ):
            raise ValueError("The pinned curl distribution differs")
        stage = "curl_capabilities"
        manifest.update(
            revision=revision,
            tracked_status=tracked,
            curl_provenance=provenance,
            source_sha256={
                name: common.digest(HERE.parent / name) for name in SOURCE_FILES
            },
            curl_version=subprocess.check_output(
                [curl, "--disable", "--version"], text=True, timeout=10
            ).strip(),
        )
        if (
            not manifest["curl_version"].startswith("curl 8.23.0 ")
            or "nghttp2/1.70.0" not in manifest["curl_version"]
            or "HTTP2" not in manifest["curl_version"]
        ):
            raise ValueError("The curl version or HTTP/2 support differs")
        stage = "proxy_configuration"
        proxy = manifest["system_proxy_observation"]
        if (
            proxy["proxy_enabled"]
            or proxy["proxy_server_nonempty"]
            or proxy["read_errors"]
            or any(manifest["proxy_environment_nonempty"].values())
        ):
            raise ValueError("The live proxy configuration is present or unverified")
        stage = "native_blocked_call_control"
        deadline_work = workspace / "deadline-control"
        deadline_work.mkdir()
        manifest["native_blocked_call_control"] = run_process_tree(
            [sys.executable, str(Path(__file__).resolve()), "native-blocker"],
            dict(os.environ),
            deadline_work,
            deadline_work / "stdout.log",
            deadline_work / "stderr.log",
            3,
        )
        deadline = manifest["native_blocked_call_control"]
        write_json(output / "manifest.json", manifest)
        if (
            not deadline["timed_out"]
            or deadline["job_processes"] < 2
            or not deadline["tree_exit_confirmed"]
            or deadline["active_processes_after_wait"]
        ):
            raise ValueError("The blocked native-call deadline control failed")
        stage = "native_preflight_controls"
        preflights = output / "preflights"
        preflights.mkdir()
        manifest["native_preflight_supervisor"] = run_process_tree(
            [
                sys.executable,
                str(Path(__file__).resolve()),
                "preflight-worker",
                "--curl",
                str(curl),
                "--output",
                str(preflights),
            ],
            dict(os.environ),
            workspace,
            workspace / "preflight-stdout.log",
            workspace / "preflight-stderr.log",
            90,
        )
        path = preflights / "result.json"
        manifest["native_preflights"] = (
            json.loads(path.read_text()) if path.is_file() else {"status": "missing"}
        )
        write_json(output / "manifest.json", manifest)
        if (
            manifest["native_preflight_supervisor"]["exit_code"] != 0
            or manifest["native_preflights"]["status"] != "passed"
            or not manifest["native_preflight_supervisor"]["tree_exit_confirmed"]
        ):
            raise ValueError("A required native gate did not pass")
        manifest["status"] = "running"
        write_json(output / "manifest.json", manifest)
        stage = "live_protocol_pairs"
        for index, first in enumerate(PROTOCOL["sequences"][str(args.runner_index)], 1):
            pair = run_pair(curl, output, workspace, index, first)
            manifest["pairs"].append(pair)
            write_json(output / "manifest.json", manifest)
            if not pair["supervisor"]["tree_exit_confirmed"]:
                raise ValueError("The pair process tree exit was not confirmed")
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
            status="stopped", failed_gate=stage, **failure_metadata(error, stage)
        )
    manifest["finished_at"] = common.timestamp()
    write_json(output / "manifest.json", manifest)
    return int(
        manifest["status"] != "completed"
        or any(
            arm["classification"]["status"] != "matched"
            for pair in manifest["pairs"]
            for arm in pair["arms"].values()
        )
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    prepare = commands.add_parser("prepare-curl")
    prepare.add_argument("--directory", type=Path, required=True)
    prepare.add_argument("--evidence", type=Path, required=True)
    worker = commands.add_parser("transfer-worker")
    worker.add_argument("configuration", type=Path)
    pair = commands.add_parser("pair-worker")
    pair.add_argument("configuration", type=Path)
    preflight = commands.add_parser("preflight-worker")
    preflight.add_argument("--curl", type=Path, required=True)
    preflight.add_argument("--output", type=Path, required=True)
    preflight.add_argument("--portable", action="store_true")
    preflight.add_argument("--case", choices=list(CONTROL_DEADLINES))
    raw = commands.add_parser("raw-receiver")
    raw.add_argument("port", type=int)
    raw.add_argument("noise_port", type=int)
    blocker = commands.add_parser("native-blocker")
    blocker.add_argument("--leaf", action="store_true")
    run = commands.add_parser("capture")
    run.add_argument("--curl", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--workspace", type=Path, required=True)
    run.add_argument("--runner-index", type=int, choices=[1, 2], required=True)
    run.add_argument("--expected-revision", required=True)
    args = parser.parse_args()
    if args.action == "prepare-curl":
        if os.environ.get("SSLKEYLOGFILE"):
            raise ValueError("TLS key logging must be absent for this capture")
        try:
            reference.prepare_curl(args.directory.resolve())
        finally:
            path = args.directory / "provenance.json"
            if path.is_file():
                write_json(args.evidence, json.loads(path.read_text()))
        return 0
    if args.action == "transfer-worker":
        configuration = json.loads(args.configuration.read_text())
        if (
            configuration.get("mode") != "live"
            or configuration["url"] != PROTOCOL["archive_url"]
            or configuration["peer"] != PROTOCOL["peer_address"]
            or configuration["port"] != 443
            or configuration.get("stats") is not True
        ):
            raise ValueError("The live transfer configuration differs")
        result = run_transfer(configuration, Path(configuration["output"]))
        return int(result["exit_code"] != 0)
    if args.action == "pair-worker":
        return pair_worker(args.configuration)
    if args.action == "preflight-worker":
        return preflight_worker(
            args.curl.resolve(), args.output.resolve(), args.portable, args.case
        )
    if args.action == "raw-receiver":
        return raw_receiver(args.port, args.noise_port)
    if args.action == "native-blocker":
        native_blocker(args.leaf)
        return 0
    return capture(args)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (
        OSError,
        ValueError,
        RuntimeError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
    ) as error:
        print(json.dumps(failure_metadata(error, "command")))
        sys.exit(1)
