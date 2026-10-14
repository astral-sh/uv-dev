"""Windows ABI boundary for the generated Windows Rust program.

Rust retains path parsing, UTF-16 handling, and packaging rules. This module
translates pointers at the OS boundary and owns each virtual process's resources.
"""

import bisect
import ctypes
import io
import os
import subprocess
import sys
from threading import RLock

from ._vm import Trap

_LIBRARIES = {}
_LIBRARY_LOCK = RLock()

# Positions of memory pointers in the imported Win32 signatures. HANDLE values
# and callback/function-pointer values are opaque and require separate handling.
POINTERS = {
    "GetFileInformationByHandle": {1},
    "ProcessPrng": {0},
    "GetModuleFileNameW": {1},
    "CommandLineToArgvW": {0, 1},
    "_wtoi64": {0},
    "DeleteFileW": {0},
    "LocalFree": {0},
    "SetFileAttributesW": {0},
    "MoveFileExW": {0, 1},
    "NtCancelIoFileEx": {1, 2},
    "GetQueuedCompletionStatusEx": {1, 3},
    "GetConsoleMode": {1},
    "GetConsoleScreenBufferInfo": {1},
    "InitOnceBeginInitialize": {0, 2, 3},
    "InitOnceComplete": {0},
    "WakeByAddressSingle": {0},
    "WakeByAddressAll": {0},
    "GetModuleHandleW": {0},
    "FormatMessageW": {1, 4, 6},
    "QueryPerformanceCounter": {0},
    "QueryPerformanceFrequency": {0},
    "GetModuleHandleExW": {1, 2},
    "GetFileInformationByHandleEx": {2},
    "GetModuleHandleA": {0},
    "GetProcAddress": {1},
    "HeapFree": {2},
    "HeapReAlloc": {2},
    "CreateFileW": {0, 3},
    "GetSystemInfo": {0},
    "WaitOnAddress": {0, 1},
    "NtReadFile": {4, 5, 7, 8},
    "NtWriteFile": {4, 5, 7, 8},
    "WriteConsoleW": {1, 3},
    "MultiByteToWideChar": {2, 4},
    "GetFullPathNameW": {0, 2, 3},
    "GetSystemTimePreciseAsFileTime": {0},
    "SetFileInformationByHandle": {2},
    "SetFileTime": {1, 2, 3},
    "SetFilePointerEx": {2},
    "CreateDirectoryW": {0, 1},
    "FindFirstFileExW": {0, 2, 4},
    "FindNextFileW": {1},
    "GetFinalPathNameByHandleW": {1},
    "GetEnvironmentVariableW": {0, 1},
    "GetCurrentDirectoryW": {1},
    "SetThreadStackGuarantee": {0},
    "GetTempPathW": {1},
    "GetTempPath2W": {1},
    "SetThreadDescription": {1},
}
SCALARS = {
    "CloseHandle",
    "RtlNtStatusToDosError",
    "WaitForSingleObject",
    "SetConsoleMode",
    "SetConsoleTextAttribute",
    "IsThreadAFiber",
    "GetFileType",
    "SwitchToThread",
    "GetCurrentThreadId",
    "GetCurrentThread",
    "GetConsoleOutputCP",
    "FindClose",
    "HeapAlloc",
}


class Host:
    def __init__(self, arguments, version="0.13.0", source_tree=None):
        if os.name != "nt":
            raise RuntimeError("The Windows ABI host requires Windows Python")
        self.pointer_size = ctypes.sizeof(ctypes.c_void_p)
        self.pointer_kind = 126 if self.pointer_size == 8 else 127
        self.pointer_mask = (1 << (self.pointer_size * 8)) - 1
        self.arguments = [os.fsdecode(value) for value in arguments]
        if any("\0" in value for value in self.arguments):
            raise ValueError("embedded null character")
        self.environment = {
            "UV_BUILD_SLIM_VERSION": version,
            "UV_BUILD_SLIM_SOURCE_TREE": source_tree
            if source_tree is not None
            else os.getcwd(),
        }
        self.stdout, self.stderr = io.BytesIO(), io.BytesIO()
        self.streams = {
            0x7FFF0001: io.BytesIO(),
            0x7FFF0002: self.stdout,
            0x7FFF0003: self.stderr,
        }
        self.libraries, self.functions = {}, {}
        self.ranges, self.bases = {}, []
        self.heap_blocks, self.handles, self.find_handles = set(), set(), set()
        self.tls, self.fls, self.fls_callbacks = {}, {}, {}
        self.exit_callbacks, self.exception_callbacks = [], []
        self.dynamic, self.module_names = {}, {}
        self.last_error = 0
        self.retained = []
        self.machine = None
        self.closed = False
        standard_error = self.native(
            "kernel32.dll", "GetStdHandle", [127], [self.pointer_kind]
        )(0xFFFFFFF4)
        console_mode = ctypes.c_uint32()
        is_console = self.native(
            "kernel32.dll",
            "GetConsoleMode",
            [self.pointer_kind, self.pointer_kind],
            [127],
        )(standard_error, ctypes.addressof(console_mode))
        # Console writes must remain live so Win32 color attributes and cursor
        # operations apply to the corresponding text, including legacy consoles.
        self.stderr_console = standard_error if is_console else None
        self.heap = self.native(
            "kernel32.dll",
            "HeapCreate",
            [127, self.pointer_kind, self.pointer_kind],
            [self.pointer_kind],
        )(0, 16 * 1024 * 1024, 0)
        if not self.heap:
            raise MemoryError("Create the generated program's private Windows heap")
        command_line = (
            subprocess.list2cmdline(["uv-build", *self.arguments]) + "\0"
        ).encode("utf-16-le", "surrogatepass")
        self.command_line = self.native(
            "kernel32.dll",
            "HeapAlloc",
            [self.pointer_kind, 127, self.pointer_kind],
            [self.pointer_kind],
        )(self.heap, 0, len(command_line))
        if not self.command_line:
            self.native("kernel32.dll", "HeapDestroy", [self.pointer_kind], [127])(
                self.heap
            )
            raise MemoryError("Allocate the generated program's command line")
        ctypes.memmove(self.command_line, command_line, len(command_line))
        self.register(self.command_line, len(command_line))
        self.heap_blocks.add(self.command_line)

    def bind(self, machine):
        self.machine = machine
        self.memory = machine.memory
        if self.memory.address_bits != self.pointer_size * 8:
            raise RuntimeError(
                "The generated Windows program and Python pointer widths differ"
            )
        self.memory.external = self
        for address in self.bases:
            if address < len(self.memory.data):
                raise Trap(
                    "Native allocation overlaps the guest linear-memory address range"
                )
        self.signatures = {
            (module, name): machine.description["types"][type_index]
            for module, name, type_index in machine.description["imports"]
        }

    def native(self, module, name, arguments, returns):
        key = module, name, tuple(arguments), tuple(returns)
        if key not in self.functions:
            if module not in self.libraries:
                with _LIBRARY_LOCK:
                    if module not in _LIBRARIES:
                        _LIBRARIES[module] = ctypes.WinDLL(
                            "msvcrt.dll" if module == "env" else module,
                            use_last_error=True,
                        )
                    self.libraries[module] = _LIBRARIES[module]
            library = self.libraries[module]
            types = {
                127: ctypes.c_uint32,
                126: ctypes.c_uint64,
                125: ctypes.c_float,
                124: ctypes.c_double,
            }
            prototype = ctypes.CFUNCTYPE if module == "env" else ctypes.WINFUNCTYPE
            function = prototype(
                types[returns[0]] if returns else None,
                *[types[kind] for kind in arguments],
                use_last_error=True,
            )((name, library))
            self.functions[key] = function
        return self.functions[key]

    def register(self, address, size):
        if not address:
            return
        if self.machine is not None and address < len(self.memory.data):
            raise Trap(
                "Native allocation overlaps the guest linear-memory address range"
            )
        if address not in self.ranges:
            bisect.insort(self.bases, address)
        self.ranges[address] = max(size, 1)

    def unregister(self, address):
        if address in self.ranges:
            del self.ranges[address]
            self.bases.remove(address)

    def validate_memory(self, address, size):
        index = bisect.bisect_right(self.bases, address) - 1
        if size < 0 or index < 0:
            raise Trap("Unregistered Windows memory")
        base = self.bases[index]
        if address + size > base + self.ranges[base]:
            raise Trap("Out of bounds Windows allocation access")

    def read_memory(self, address, size):
        self.validate_memory(address, size)
        return bytearray(ctypes.string_at(address, size))

    def write_memory(self, address, data):
        self.validate_memory(address, len(data))
        ctypes.memmove(address, bytes(data), len(data))

    def pointer(self, value):
        if value == 0:
            return 0
        if value < len(self.memory.data):
            return (
                ctypes.addressof(ctypes.c_ubyte.from_buffer(self.memory.data)) + value
            )
        self.validate_memory(value, 0)
        return value

    def put_pointer(self, address, value):
        self.memory.put(
            address, (value & self.pointer_mask).to_bytes(self.pointer_size, "little")
        )

    def string(self, address, wide):
        result = bytearray()
        width = 2 if wide else 1
        while True:
            data = self.memory.get(address, width)
            if not any(data):
                break
            result.extend(data)
            address += width
        return result.decode("utf-16-le" if wide else "ascii", "surrogatepass")

    def indirect(self, value, expected_type):
        if value not in self.dynamic:
            raise Trap("Unknown Windows function pointer")
        module, name = self.dynamic[value]
        self.signatures[module, name] = self.machine.description["types"][expected_type]
        return module, name, expected_type

    def callback(self, pointer, arguments):
        if pointer:
            function = self.machine.tables[0][pointer]
            self.machine.call(function, arguments)

    def __call__(self, module, name, context, arguments):
        if os.environ.get("UV_BUILD_SLIM_TRACE"):
            print(
                "Windows import",
                module,
                name,
                [
                    hex(value) if isinstance(value, int) else value
                    for value in arguments
                ],
                file=sys.stderr,
                flush=True,
            )
        types, returns = self.signatures[module, name]
        args = [
            value & ((1 << (32 if kind == 127 else 64)) - 1)
            if kind in {126, 127}
            else value
            for kind, value in zip(types, arguments)
        ]
        if name == "GetLastError":
            return [self.last_error]
        if name == "SetLastError":
            self.last_error = args[0]
            return []
        if name == "ExitProcess":
            raise Exception(SystemExit, args[0])
        if name == "GetProcessHeap":
            return [self.heap]
        if name == "GetCommandLineW":
            return [self.command_line]
        if name == "GetStdHandle":
            if args[0] == 0xFFFFFFF4 and self.stderr_console is not None:
                return [self.stderr_console]
            return [
                {
                    0xFFFFFFF6: 0x7FFF0001,
                    0xFFFFFFF5: 0x7FFF0002,
                    0xFFFFFFF4: 0x7FFF0003,
                }.get(args[0], 0)
            ]
        if (
            name
            in {
                "GetFileType",
                "GetConsoleMode",
                "GetConsoleScreenBufferInfo",
                "SetConsoleMode",
                "SetConsoleTextAttribute",
            }
            and args[0] in self.streams
        ):
            if name == "GetFileType":
                return [3]
            self.last_error = 6
            return [0]
        if name in {"NtWriteFile", "NtReadFile"} and args[0] in self.streams:
            stream = self.streams[args[0]]
            if name == "NtWriteFile":
                data = self.memory.get(args[5], args[6])
                stream.write(data)
            else:
                data = stream.read(args[6])
                self.memory.put(args[5], data)
            self.put_pointer(args[4], 0)
            self.put_pointer(args[4] + self.pointer_size, len(data))
            return [0]
        if name in {"TlsAlloc", "FlsAlloc"}:
            values = self.tls if name == "TlsAlloc" else self.fls
            index = next(
                index for index in range(len(values) + 1) if index not in values
            )
            values[index] = 0
            if name == "FlsAlloc":
                self.fls_callbacks[index] = args[0]
            return [index]
        if name in {
            "TlsGetValue",
            "TlsSetValue",
            "TlsFree",
            "FlsGetValue",
            "FlsSetValue",
            "FlsFree",
        }:
            values = self.tls if name.startswith("Tls") else self.fls
            if name.endswith("GetValue"):
                self.last_error = 0 if args[0] in values else 87
                return [values.get(args[0], 0)]
            if args[0] not in values:
                self.last_error = 87
                return [0]
            if name.endswith("SetValue"):
                values[args[0]] = args[1]
            else:
                value = values.pop(args[0])
                if name == "FlsFree":
                    callback = self.fls_callbacks.pop(args[0], 0)
                    if value:
                        self.callback(callback, [value])
            return [1]
        if name == "atexit":
            self.exit_callbacks.append(args[0])
            return [0]
        if name == "AddVectoredExceptionHandler":
            self.exception_callbacks.append(args[1])
            return [len(self.exception_callbacks)]
        if name == "SetThreadStackGuarantee":
            self.memory.put_u32(args[0], 0)
            return [1]
        if name == "GetModuleHandleExW" and args[0] & 4:
            handle = self.native(
                "kernel32.dll",
                "GetModuleHandleW",
                [self.pointer_kind],
                [self.pointer_kind],
            )(0)
            self.put_pointer(args[2], handle)
            return [1]
        if name == "GetEnvironmentVariableW":
            key = self.string(args[0], True)
            if key in self.environment:
                data = (
                    self.environment[key].encode("utf-16-le", "surrogatepass") + b"\0\0"
                )
                if args[2] * 2 >= len(data):
                    self.memory.put(args[1], data)
                    return [len(data) // 2 - 1]
                return [len(data) // 2]
        if name == "CreateProcessA":
            raise NotImplementedError(
                "Child process creation requires a separate virtual-process adapter"
            )
        if name not in POINTERS and name not in SCALARS:
            raise Trap("Unmapped Windows ABI function: " + module + "!" + name)
        positions = POINTERS.get(name, set())
        converted = [
            self.pointer(value) if index in positions else value
            for index, value in enumerate(args)
        ]
        if name == "HeapFree" and args[2] and args[2] not in self.heap_blocks:
            raise Trap("Attempt to free an unowned Windows allocation")
        ctypes.set_last_error(self.last_error)
        function = self.native(module, name, types, returns)
        result = function(*converted)
        self.last_error = ctypes.get_last_error()
        if name in {"HeapAlloc", "HeapReAlloc"} and result:
            if name == "HeapReAlloc":
                self.unregister(args[2])
                self.heap_blocks.discard(args[2])
            self.register(result, args[2] if name == "HeapAlloc" else args[3])
            self.heap_blocks.add(result)
        elif name == "HeapFree" and result:
            self.unregister(args[2])
            self.heap_blocks.discard(args[2])
        elif name == "CommandLineToArgvW" and result:
            size = self.native(
                "kernel32.dll", "LocalSize", [self.pointer_kind], [self.pointer_kind]
            )(result)
            self.register(result, size)
        elif name == "LocalFree" and not result:
            self.unregister(args[0])
        elif name == "GetProcAddress" and result:
            token = 0x40000000 + len(self.dynamic)
            self.dynamic[token] = (
                self.module_names.get(args[0], "kernel32.dll"),
                self.string(args[1], False),
            )
            result = token
        elif name in {"GetModuleHandleW", "GetModuleHandleA"} and result and args[0]:
            self.module_names[result] = self.string(args[0], name.endswith("W"))
        elif name == "CreateFileW" and result not in {0, self.pointer_mask}:
            self.handles.add(result)
        elif name == "CloseHandle" and result:
            self.handles.discard(args[0])
        elif name == "FindFirstFileExW" and result not in {0, self.pointer_mask}:
            self.find_handles.add(result)
        elif name == "FindClose" and result:
            self.find_handles.discard(args[0])
        return [result] if returns else []

    def close(self):
        if self.closed:
            return
        self.closed = True
        try:
            if self.machine is not None:
                for callback in reversed(self.exit_callbacks):
                    self.callback(callback, [])
                for index, value in list(self.fls.items()):
                    if value:
                        self.fls[index] = 0
                        self.callback(self.fls_callbacks.get(index, 0), [value])
        finally:
            for handle in list(self.find_handles):
                self.native("kernel32.dll", "FindClose", [self.pointer_kind], [127])(
                    handle
                )
            for handle in list(self.handles):
                self.native("kernel32.dll", "CloseHandle", [self.pointer_kind], [127])(
                    handle
                )
            self.native("kernel32.dll", "HeapDestroy", [self.pointer_kind], [127])(
                self.heap
            )
