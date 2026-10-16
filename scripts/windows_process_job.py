"""Run a Windows process tree with a deadline and wait for every job member."""

import ctypes
import os
import subprocess
import time
from pathlib import Path

DWORD = ctypes.c_uint32
HANDLE = ctypes.c_void_p
SIZE_T = ctypes.c_size_t


class BasicLimits(ctypes.Structure):
    _fields_ = [
        ("process_time", ctypes.c_int64),
        ("job_time", ctypes.c_int64),
        ("flags", DWORD),
        ("minimum_working_set", SIZE_T),
        ("maximum_working_set", SIZE_T),
        ("active_process_limit", DWORD),
        ("affinity", SIZE_T),
        ("priority_class", DWORD),
        ("scheduling_class", DWORD),
    ]


class ExtendedLimits(ctypes.Structure):
    _fields_ = [
        ("basic", BasicLimits),
        ("io_counters", ctypes.c_uint64 * 6),
        ("process_memory_limit", SIZE_T),
        ("job_memory_limit", SIZE_T),
        ("peak_process_memory", SIZE_T),
        ("peak_job_memory", SIZE_T),
    ]


class Accounting(ctypes.Structure):
    _fields_ = [
        ("total_user_time", ctypes.c_int64),
        ("total_kernel_time", ctypes.c_int64),
        ("period_user_time", ctypes.c_int64),
        ("period_kernel_time", ctypes.c_int64),
        ("page_fault_count", DWORD),
        ("total_processes", DWORD),
        ("active_processes", DWORD),
        ("terminated_processes", DWORD),
    ]


class StartupInfo(ctypes.Structure):
    _fields_ = [
        ("size", DWORD),
        ("reserved", ctypes.c_wchar_p),
        ("desktop", ctypes.c_wchar_p),
        ("title", ctypes.c_wchar_p),
        ("x", DWORD),
        ("y", DWORD),
        ("width", DWORD),
        ("height", DWORD),
        ("character_width", DWORD),
        ("character_height", DWORD),
        ("fill_attribute", DWORD),
        ("flags", DWORD),
        ("show_window", ctypes.c_uint16),
        ("reserved_size", ctypes.c_uint16),
        ("reserved_bytes", ctypes.c_void_p),
        ("stdin", HANDLE),
        ("stdout", HANDLE),
        ("stderr", HANDLE),
    ]


class ProcessInfo(ctypes.Structure):
    _fields_ = [
        ("process", HANDLE),
        ("thread", HANDLE),
        ("pid", DWORD),
        ("thread_id", DWORD),
    ]


def run_process_tree(
    arguments, environment, cwd, stdout_path, stderr_path, timeout_seconds
):
    if os.name != "nt":
        raise RuntimeError("The process-tree supervisor requires Windows")
    import msvcrt

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    declarations = {
        "CreateJobObjectW": ([ctypes.c_void_p, ctypes.c_wchar_p], HANDLE),
        "SetInformationJobObject": (
            [HANDLE, ctypes.c_int, ctypes.c_void_p, DWORD],
            ctypes.c_int32,
        ),
        "QueryInformationJobObject": (
            [HANDLE, ctypes.c_int, ctypes.c_void_p, DWORD, ctypes.c_void_p],
            ctypes.c_int32,
        ),
        "AssignProcessToJobObject": ([HANDLE, HANDLE], ctypes.c_int32),
        "TerminateJobObject": ([HANDLE, DWORD], ctypes.c_int32),
        "TerminateProcess": ([HANDLE, DWORD], ctypes.c_int32),
        "ResumeThread": ([HANDLE], DWORD),
        "GetExitCodeProcess": ([HANDLE, ctypes.POINTER(DWORD)], ctypes.c_int32),
        "WaitForSingleObject": ([HANDLE, DWORD], DWORD),
        "CloseHandle": ([HANDLE], ctypes.c_int32),
        "CreateProcessW": (
            [
                ctypes.c_wchar_p,
                ctypes.c_wchar_p,
                ctypes.c_void_p,
                ctypes.c_void_p,
                ctypes.c_int32,
                DWORD,
                ctypes.c_void_p,
                ctypes.c_wchar_p,
                ctypes.POINTER(StartupInfo),
                ctypes.POINTER(ProcessInfo),
            ],
            ctypes.c_int32,
        ),
    }
    for name, (parameters, result) in declarations.items():
        function = getattr(kernel, name)
        function.argtypes = parameters
        function.restype = result

    def check(result):
        if not result:
            raise ctypes.WinError(ctypes.get_last_error())
        return result

    def accounting():
        result = Accounting()
        check(
            kernel.QueryInformationJobObject(
                job, 1, ctypes.byref(result), ctypes.sizeof(result), None
            )
        )
        return result

    job = check(kernel.CreateJobObjectW(None, None))
    process = ProcessInfo()
    assigned = False
    started = time.monotonic()
    deadline = started + timeout_seconds
    try:
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        check(
            kernel.SetInformationJobObject(
                job, 9, ctypes.byref(limits), ctypes.sizeof(limits)
            )
        )
        with (
            open(os.devnull, "rb") as stdin,
            Path(stdout_path).open("wb") as stdout,
            Path(stderr_path).open("wb") as stderr,
        ):
            handles = [
                msvcrt.get_osfhandle(stream.fileno())
                for stream in (stdin, stdout, stderr)
            ]
            startup = StartupInfo()
            startup.size = ctypes.sizeof(startup)
            startup.flags = 0x100  # STARTF_USESTDHANDLES
            startup.stdin, startup.stdout, startup.stderr = handles
            command = ctypes.create_unicode_buffer(
                subprocess.list2cmdline([str(value) for value in arguments])
            )
            environment_block = ctypes.create_unicode_buffer(
                "\0".join(
                    f"{name}={value}"
                    for name, value in sorted(
                        environment.items(), key=lambda item: item[0].upper()
                    )
                )
                + "\0\0"
            )
            for handle in handles:
                os.set_handle_inheritable(handle, True)
            try:
                check(
                    kernel.CreateProcessW(
                        str(arguments[0]),
                        command,
                        None,
                        None,
                        True,
                        0x4 | 0x400,  # CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT
                        ctypes.cast(environment_block, ctypes.c_void_p),
                        str(cwd),
                        ctypes.byref(startup),
                        ctypes.byref(process),
                    )
                )
            finally:
                for handle in handles:
                    os.set_handle_inheritable(handle, False)
            # Assign before resuming so normal child processes inherit the job.
            check(kernel.AssignProcessToJobObject(job, process.process))
            assigned = True
            if kernel.ResumeThread(process.thread) == 0xFFFFFFFF:
                raise ctypes.WinError(ctypes.get_last_error())
            check(kernel.CloseHandle(process.thread))
            process.thread = None
            timed_out = False
            while accounting().active_processes:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    check(kernel.TerminateJobObject(job, 124))
                    break
                time.sleep(min(0.02, remaining))
            cleanup_deadline = time.monotonic() + 10
            while accounting().active_processes:
                if time.monotonic() >= cleanup_deadline:
                    raise RuntimeError(
                        "Job members remain after termination; refusing another trial"
                    )
                time.sleep(0.02)
            if kernel.WaitForSingleObject(process.process, 1000) != 0:
                raise RuntimeError(
                    "Root process exit was not signaled after the job became empty"
                )
            exit_code = DWORD()
            check(kernel.GetExitCodeProcess(process.process, ctypes.byref(exit_code)))
            final = accounting()
            return {
                "pid": process.pid,
                "exit_code": exit_code.value,
                "timed_out": timed_out,
                "elapsed_seconds": time.monotonic() - started,
                "job_processes": final.total_processes,
                "active_processes_after_wait": final.active_processes,
                "tree_exit_confirmed": True,
            }
    finally:
        if process.process:
            if not assigned:
                kernel.TerminateProcess(process.process, 125)
                kernel.WaitForSingleObject(process.process, 10000)
            kernel.CloseHandle(process.process)
        if process.thread:
            kernel.CloseHandle(process.thread)
        kernel.CloseHandle(job)
