"""Read receiver statistics for a live, owned Windows TCP connection."""

import ctypes
import datetime
import os
import socket
import struct
import time

ULONG = ctypes.c_uint32
ULONG64 = ctypes.c_uint64
BOOLEAN = ctypes.c_ubyte
SIZE_T = ctypes.c_uint64


class TcpRow(ctypes.Structure):
    _fields_ = [
        ("state", ULONG),
        ("local_address", ULONG),
        ("local_port", ULONG),
        ("remote_address", ULONG),
        ("remote_port", ULONG),
    ]


class OwnedTcpRow(ctypes.Structure):
    _fields_ = TcpRow._fields_ + [("pid", ULONG)]


class Collection(ctypes.Structure):
    _fields_ = [("enabled", BOOLEAN)]


class Receiver(ctypes.Structure):
    _fields_ = [
        (name, ULONG)
        for name in [
            "CurRwinSent",
            "MaxRwinSent",
            "MinRwinSent",
            "LimRwin",
            "DupAckEpisodes",
            "DupAcksOut",
            "CeRcvd",
            "EcnSent",
            "EcnNoncesRcvd",
            "CurReasmQueue",
            "MaxReasmQueue",
        ]
    ] + [
        ("CurAppRQueue", SIZE_T),
        ("MaxAppRQueue", SIZE_T),
        ("WinScaleSent", BOOLEAN),
    ]


class Data(ctypes.Structure):
    _fields_ = (
        [
            (name, ULONG64)
            for name in [
                "DataBytesOut",
                "DataSegsOut",
                "DataBytesIn",
                "DataSegsIn",
                "SegsOut",
                "SegsIn",
            ]
        ]
        + [
            (name, ULONG)
            for name in ["SoftErrors", "SoftErrorReason", "SndUna", "SndNxt", "SndMax"]
        ]
        + [
            ("ThruBytesAcked", ULONG64),
            ("RcvNxt", ULONG),
            ("ThruBytesReceived", ULONG64),
        ]
    )


class NetworkPath(ctypes.Structure):
    _fields_ = [
        (name, ULONG)
        for name in [
            "FastRetran",
            "Timeouts",
            "SubsequentTimeouts",
            "CurTimeoutCount",
            "AbruptTimeouts",
            "PktsRetrans",
            "BytesRetrans",
            "DupAcksIn",
            "SacksRcvd",
            "SackBlocksRcvd",
            "CongSignals",
            "PreCongSumCwnd",
            "PreCongSumRtt",
            "PostCongSumRtt",
            "PostCongCountRtt",
            "EcnSignals",
            "EceRcvd",
            "SendStall",
            "QuenchRcvd",
            "RetranThresh",
            "SndDupAckEpisodes",
            "SumBytesReordered",
            "NonRecovDa",
            "NonRecovDaEpisodes",
            "AckAfterFr",
            "DsackDups",
            "SampleRtt",
            "SmoothedRtt",
            "RttVar",
            "MaxRtt",
            "MinRtt",
            "SumRtt",
            "CountRtt",
            "CurRto",
            "MaxRto",
            "MinRto",
            "CurMss",
            "MaxMss",
            "MinMss",
            "SpuriousRtoDetections",
        ]
    ]


CATEGORIES = {
    "receiver": (
        5,
        Receiver,
        [
            "CurRwinSent",
            "MaxRwinSent",
            "MinRwinSent",
            "LimRwin",
            "WinScaleSent",
            "DupAckEpisodes",
            "DupAcksOut",
            "CeRcvd",
            "CurReasmQueue",
            "MaxReasmQueue",
            "CurAppRQueue",
            "MaxAppRQueue",
        ],
    ),
    "data": (
        1,
        Data,
        [
            "DataBytesIn",
            "DataSegsIn",
            "ThruBytesReceived",
            "SoftErrors",
            "SoftErrorReason",
        ],
    ),
    "path": (
        3,
        NetworkPath,
        [
            "SampleRtt",
            "SmoothedRtt",
            "MinRtt",
            "MaxRtt",
            "CountRtt",
            "CurRto",
        ],
    ),
}


def abi_layout():
    return {
        structure.__name__: {
            "size": ctypes.sizeof(structure),
            "alignment": ctypes.alignment(structure),
            "offsets": {
                name: getattr(structure, name).offset
                for name, _kind in structure._fields_
            },
        }
        for structure in [TcpRow, OwnedTcpRow, Collection, Receiver, Data, NetworkPath]
    }


def address(value):
    return socket.inet_ntoa(struct.pack("<I", value))


def port(value):
    return socket.ntohs(value & 0xFFFF)


def owned_rows(buffer, pid, remote_address, remote_port):
    """Copy only rows for the owned process and the assigned endpoint."""
    if len(buffer) < 4:
        raise ValueError("TCP table has no count")
    count = struct.unpack_from("<I", buffer)[0]
    row_size = ctypes.sizeof(OwnedTcpRow)
    if count > (len(buffer) - 4) // row_size:
        raise ValueError("TCP table exceeds its returned buffer")
    selected = []
    for index in range(count):
        row = OwnedTcpRow.from_buffer_copy(buffer, 4 + index * row_size)
        if (
            row.pid == pid
            and address(row.remote_address) == remote_address
            and port(row.remote_port) == remote_port
            and row.state in (5, 8)  # `ESTABLISHED` or `CLOSE_WAIT`
        ):
            selected.append(TcpRow.from_buffer_copy(row))
    return selected


def connection_identity(row):
    return {
        "local_address": address(row.local_address),
        "local_port": port(row.local_port),
        "remote_address": address(row.remote_address),
        "remote_port": port(row.remote_port),
    }


class StatsError(RuntimeError):
    def __init__(self, operation, code):
        self.operation = operation
        self.code = int(code)
        super().__init__(f"{operation} returned {code}")


class WindowsStats:
    """Keep the child process handle alive while attributing connection rows."""

    def __init__(self, process, remote_address, remote_port):
        if os.name != "nt" or ctypes.sizeof(ctypes.c_void_p) != 8:
            raise RuntimeError("The receiver sampler requires 64-bit Windows")
        self.process = process
        self.remote_address = remote_address
        self.remote_port = remote_port
        self.handle = ctypes.c_void_p(int(process._handle))
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ULONG]
        self.kernel.WaitForSingleObject.restype = ULONG
        self.kernel.GetProcessId.argtypes = [ctypes.c_void_p]
        self.kernel.GetProcessId.restype = ULONG
        self.kernel.GetProcessTimes.argtypes = [
            ctypes.c_void_p,
            *([ctypes.POINTER(ULONG64)] * 4),
        ]
        self.kernel.GetProcessTimes.restype = ctypes.c_int32
        self.library = ctypes.WinDLL("iphlpapi", use_last_error=True)
        self.library.GetExtendedTcpTable.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ULONG),
            ctypes.c_int32,
            ULONG,
            ctypes.c_int32,
            ULONG,
        ]
        self.library.GetExtendedTcpTable.restype = ULONG
        self.library.SetPerTcpConnectionEStats.argtypes = [
            ctypes.POINTER(TcpRow),
            ctypes.c_int32,
            ctypes.c_void_p,
            ULONG,
            ULONG,
            ULONG,
        ]
        self.library.SetPerTcpConnectionEStats.restype = ULONG
        self.library.GetPerTcpConnectionEStats.argtypes = [
            ctypes.POINTER(TcpRow),
            ctypes.c_int32,
            ctypes.c_void_p,
            ULONG,
            ULONG,
            ctypes.c_void_p,
            ULONG,
            ULONG,
            ctypes.c_void_p,
            ULONG,
            ULONG,
        ]
        self.library.GetPerTcpConnectionEStats.restype = ULONG
        self.creation_time = self.process_identity()
        self.enabled = set()
        self.generations = {}
        self.next_generation = 1

    def process_identity(self):
        values = [ULONG64() for _ in range(4)]
        if self.kernel.GetProcessId(self.handle) != self.process.pid:
            raise RuntimeError("Owned process handle has a different PID")
        if not self.kernel.GetProcessTimes(
            self.handle, *(ctypes.byref(value) for value in values)
        ):
            raise StatsError("GetProcessTimes", ctypes.get_last_error())
        return values[0].value

    def alive(self):
        state = self.kernel.WaitForSingleObject(self.handle, 0)
        if state == 0:
            return False
        if state != 258:  # `WAIT_TIMEOUT`
            raise StatsError(
                "WaitForSingleObject",
                ctypes.get_last_error() if state == 0xFFFFFFFF else state,
            )
        return self.process_identity() == self.creation_time

    def rows(self):
        if not self.alive():
            return []
        size = ULONG(0)
        status = self.library.GetExtendedTcpTable(
            None, ctypes.byref(size), False, 2, 5, 0
        )
        for _ in range(3):
            if status != 122 or not 4 <= size.value <= 16 * 1024 * 1024:
                raise StatsError("GetExtendedTcpTable.size", status)
            buffer = ctypes.create_string_buffer(size.value)
            status = self.library.GetExtendedTcpTable(
                buffer, ctypes.byref(size), False, 2, 5, 0
            )
            if status == 0:
                if not self.alive():
                    return []
                return owned_rows(
                    buffer.raw[: size.value],
                    self.process.pid,
                    self.remote_address,
                    self.remote_port,
                )
        raise StatsError("GetExtendedTcpTable", status)

    def sample(self):
        started = time.monotonic()
        sample = {
            "started_monotonic": started,
            "started_at_utc": datetime.datetime.now(datetime.UTC).isoformat(),
            "status": "live",
            "observations": [],
        }

        def finish(status):
            sample.update(
                status=status,
                finished_monotonic=time.monotonic(),
                finished_at_utc=datetime.datetime.now(datetime.UTC).isoformat(),
            )
            return sample

        rows = self.rows()
        if not self.alive():
            return finish("owned_process_exited")
        present = {tuple(connection_identity(row).values()) for row in rows}
        disappeared = set(self.generations) - present
        sample["disappeared_generations"] = [
            self.generations[key] for key in disappeared
        ]
        for key in disappeared:
            del self.generations[key]
            self.enabled = {entry for entry in self.enabled if entry[0] != key}
        for row in rows:
            identity = connection_identity(row)
            key = tuple(identity.values())
            if key not in self.generations:
                self.generations[key] = self.next_generation
                self.next_generation += 1
            observation = {
                "connection": identity,
                "pid": self.process.pid,
                "process_creation_time": self.creation_time,
                "categories": {},
                "observed_generation": self.generations[key],
            }
            sample["observations"].append(observation)
            for name, (category, structure, fields) in CATEGORIES.items():
                if not self.alive():
                    return finish("owned_process_exited_during_sample")
                result = {"started_monotonic": time.monotonic()}
                observation["categories"][name] = result
                if (key, category) not in self.enabled:
                    enabled = Collection()
                    probe = self.library.GetPerTcpConnectionEStats(
                        ctypes.byref(row),
                        category,
                        ctypes.byref(enabled),
                        0,
                        ctypes.sizeof(enabled),
                        None,
                        0,
                        0,
                        None,
                        0,
                        0,
                    )
                    result.update(
                        enable_probe_status=int(probe),
                        initially_enabled=bool(enabled.enabled),
                    )
                    if not self.alive():
                        return finish("owned_process_exited_during_sample")
                    if probe == 0 and enabled.enabled:
                        self.enabled.add((key, category))
                    elif probe == 0:
                        enabled = Collection(1)
                        status = self.library.SetPerTcpConnectionEStats(
                            ctypes.byref(row),
                            category,
                            ctypes.byref(enabled),
                            0,
                            ctypes.sizeof(enabled),
                            0,
                        )
                        result["enable_status"] = int(status)
                        if status == 0:
                            self.enabled.add((key, category))
                    if not self.alive():
                        return finish("owned_process_exited_during_sample")
                enabled = Collection()
                values = structure()
                status = self.library.GetPerTcpConnectionEStats(
                    ctypes.byref(row),
                    category,
                    ctypes.byref(enabled),
                    0,
                    ctypes.sizeof(enabled),
                    None,
                    0,
                    0,
                    ctypes.byref(values),
                    0,
                    ctypes.sizeof(values),
                )
                result.update(
                    status=int(status),
                    enabled=bool(enabled.enabled),
                    finished_monotonic=time.monotonic(),
                )
                if status == 0 and enabled.enabled and (key, category) in self.enabled:
                    result["values"] = {
                        field: int(getattr(values, field)) for field in fields
                    }
                elif status == 0 and not enabled.enabled:
                    observation["collection_continuity_changed"] = True
                    self.enabled.discard((key, category))
                    self.generations.pop(key, None)
                if not self.alive():
                    return finish("owned_process_exited_during_sample")
        return finish("live")
