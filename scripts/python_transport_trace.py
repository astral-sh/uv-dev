"""Retain bounded numeric HTTP/2 events and allowlisted metadata from `curl` diagnostics."""

import json
import queue
import re
import threading
import time

MAX_EVENTS = 32768
MAX_BYTES = 4 * 1024 * 1024
MAX_LINE = 16384
PREFIX = re.compile(
    rb"^(\d{2}):(\d{2}):(\d{2})\.(\d{6}) "
    rb"\[(\d{1,19})-(\d{1,19}|x)\] ([*<>}{]) (.*)$"
)
H2_PREFIX = re.compile(rb"^\[HTTP/2\] \[(\d{1,10})\] (.*)$")
PATTERNS = [
    (
        1,
        re.compile(rb"^(<-|->) FRAME\[DATA, len=(\d+), eos=([01]), padlen=(\d+)\]$"),
        ["direction", "length", "end_stream", "padding_length"],
    ),
    (
        2,
        re.compile(rb"^(<-|->) FRAME\[WINDOW_UPDATE, incr=(\d+)\]$"),
        ["direction", "increment"],
    ),
    (
        3,
        re.compile(rb"^local window update by (-?\d+)$"),
        ["increment"],
    ),
    (
        4,
        re.compile(rb"^local window size now (-?\d+)$"),
        ["effective_window"],
    ),
    (
        5,
        re.compile(
            rb"^cf_recv\(len=(\d+)\) -> (-?\d+), (\d+), "
            rb"window=(-?\d+)/(-?\d+), connection (-?\d+)/(-?\d+)$"
        ),
        [
            "requested_bytes",
            "result",
            "read_bytes",
            "effective_received",
            "effective_window",
            "connection_credit",
            "connection_maximum",
        ],
    ),
    (
        6,
        re.compile(
            rb"^(<-|->) FRAME\[RST_STREAM, len=(\d+), flags=(\d+), error=(\d+)\]$"
        ),
        ["direction", "length", "flags", "error"],
    ),
    (
        7,
        re.compile(rb"^DATA, window=(-?\d+)/(-?\d+)$"),
        ["effective_received", "effective_window"],
    ),
]
SELECTED_PREFIXES = (
    b"<- FRAME[DATA",
    b"-> FRAME[DATA",
    b"<- FRAME[WINDOW_UPDATE",
    b"-> FRAME[WINDOW_UPDATE",
    b"local window",
    b"cf_recv(",
    b"<- FRAME[RST_STREAM",
    b"-> FRAME[RST_STREAM",
    b"DATA, window=",
)
HEADER_VALUES = {
    "cf-ray": re.compile(r"[0-9a-f]{16}-[A-Z]{3}"),
    "cf-cache-status": re.compile(
        r"HIT|MISS|EXPIRED|STALE|BYPASS|DYNAMIC|REVALIDATED|UPDATING"
    ),
    "content-length": re.compile(r"[0-9]{1,12}"),
    "age": re.compile(r"[0-9]{1,12}"),
}


def numeric_event(line, observed_monotonic):
    prefix = PREFIX.fullmatch(line.rstrip(b"\r\n"))
    if prefix is None or prefix[7] != b"*":
        return None, False
    message = H2_PREFIX.fullmatch(prefix[8])
    if message is None:
        return None, False
    for kind, pattern, fields in PATTERNS:
        values = pattern.fullmatch(message[2])
        if values is None:
            continue
        event = {
            "event": kind,
            "observed_monotonic": observed_monotonic,
            "trace_seconds_of_day": (
                int(prefix[1]) * 3600
                + int(prefix[2]) * 60
                + int(prefix[3])
                + int(prefix[4]) / 1_000_000
            ),
            "transfer_id": int(prefix[5]),
            "connection_id": int(prefix[6]) if prefix[6] != b"x" else -1,
            "stream_id": int(message[1]),
        }
        for field, value in zip(fields, values.groups(), strict=True):
            event[field] = int(value == b"->") if field == "direction" else int(value)
        return event, False
    return None, message[2].startswith(SELECTED_PREFIXES)


class TraceDrain:
    """Keep the pipe reader independent of disk writes and native counter calls."""

    def __init__(
        self, stream, canaries=None, *, max_events=MAX_EVENTS, max_bytes=MAX_BYTES
    ):
        self.stream = stream
        self.canaries = canaries or {}
        self.max_events = max_events
        self.max_bytes = max_bytes
        self.events = queue.Queue(maxsize=8192)
        self.lock = threading.Lock()
        self.record = {
            "diagnostic_bytes": 0,
            "lines": 0,
            "accepted_events": 0,
            "accepted_bytes": 0,
            "queue_high_water": 0,
            "dropped_events": 0,
            "oversize_lines": 0,
            "selected_parse_errors": 0,
            "read_calls": 0,
            "maximum_read_gap_seconds": 0,
            "maximum_parse_seconds": 0,
            "canaries_seen": {},
            "response_headers": {},
            "response_statuses": [],
            "response_header_events": [],
            "connection_ids": [],
            "dropped_header_events": 0,
            "finished": False,
        }
        self.thread = threading.Thread(target=self.run, daemon=True)

    def start(self):
        self.thread.start()

    def snapshot(self):
        with self.lock:
            return json.loads(json.dumps(self.record))

    def take(self):
        lines = []
        while True:
            try:
                lines.append(self.events.get_nowait())
            except queue.Empty:
                return lines

    def line(self, line):
        observed = time.monotonic()
        event, invalid = numeric_event(line, observed)
        prefix = PREFIX.fullmatch(line.rstrip(b"\r\n"))
        with self.lock:
            self.record["lines"] += 1
            self.record["selected_parse_errors"] += int(invalid)
            if prefix is not None:
                connection = int(prefix[6]) if prefix[6] != b"x" else -1
                if (
                    connection >= 0
                    and connection not in self.record["connection_ids"]
                    and len(self.record["connection_ids"]) < 16
                ):
                    self.record["connection_ids"].append(connection)
                if prefix[7] == b"<":
                    text = prefix[8].decode("ascii", errors="replace")
                    status = re.fullmatch(
                        r"HTTP/(1\.1|2) ([1-5][0-9]{2})(?: [A-Za-z ]*)?", text
                    )
                    if status:
                        self.record["response_headers"] = {}
                        if len(self.record["response_statuses"]) < 32:
                            self.record["response_statuses"].append(int(status[2]))
                            self.record["response_header_events"].append(
                                {
                                    "observed_monotonic": observed,
                                    "status": int(status[2]),
                                    "protocol": 2 if status[1] == "2" else 1,
                                }
                            )
                        else:
                            self.record["dropped_header_events"] += 1
                    elif ":" in text:
                        name, value = text.split(":", 1)
                        name, value = name.lower(), value.strip()
                        if name in HEADER_VALUES and HEADER_VALUES[name].fullmatch(
                            value
                        ):
                            self.record["response_headers"][name] = value
            if event is not None:
                encoded = (json.dumps(event, separators=(",", ":")) + "\n").encode()
                if (
                    self.record["accepted_events"] >= self.max_events
                    or self.record["accepted_bytes"] + len(encoded) > self.max_bytes
                ):
                    self.record["dropped_events"] += 1
                else:
                    try:
                        self.events.put_nowait(encoded)
                    except queue.Full:
                        self.record["dropped_events"] += 1
                    else:
                        self.record["accepted_events"] += 1
                        self.record["accepted_bytes"] += len(encoded)
                        self.record["queue_high_water"] = max(
                            self.record["queue_high_water"], self.events.qsize()
                        )
            self.record["maximum_parse_seconds"] = max(
                self.record["maximum_parse_seconds"], time.monotonic() - observed
            )

    def run(self):
        cpu_started = time.thread_time()
        pending = b""
        discarding = False
        canary_tail = b""
        longest_canary = max(
            (len(value) for value in self.canaries.values()), default=1
        )
        previous = time.monotonic()
        try:
            while chunk := self.stream.read1(65536):
                now = time.monotonic()
                with self.lock:
                    self.record["diagnostic_bytes"] += len(chunk)
                    self.record["read_calls"] += 1
                    self.record["maximum_read_gap_seconds"] = max(
                        self.record["maximum_read_gap_seconds"], now - previous
                    )
                    scan = canary_tail + chunk
                    for name, value in self.canaries.items():
                        if value in scan:
                            self.record["canaries_seen"][name] = True
                    canary_tail = (
                        scan[-(longest_canary - 1) :] if longest_canary > 1 else b""
                    )
                previous = now
                pending += chunk
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    if discarding:
                        discarding = False
                    elif len(line) <= MAX_LINE:
                        self.line(line)
                    else:
                        with self.lock:
                            self.record["oversize_lines"] += 1
                if len(pending) > MAX_LINE:
                    pending = b""
                    discarding = True
                    with self.lock:
                        self.record["oversize_lines"] += 1
            if pending and not discarding:
                self.line(pending)
        except (OSError, ValueError) as error:
            with self.lock:
                self.record["reader_error_type"] = type(error).__name__
        finally:
            with self.lock:
                self.record.update(
                    finished=True, thread_cpu_seconds=time.thread_time() - cpu_started
                )
