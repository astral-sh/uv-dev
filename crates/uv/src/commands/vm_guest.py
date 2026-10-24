"""Run the command from the job disk as PID 1's child in a prepared Linux image."""

import ctypes
import errno
import json
import os
import subprocess
import sys
import traceback

# Linux reboot(2) LINUX_REBOOT_CMD_POWER_OFF.
POWER_OFF = 0x4321FEDC


def main():
    libc = ctypes.CDLL(None, use_errno=True)
    for filesystem in (b"proc", b"sysfs"):
        path = b"/proc" if filesystem == b"proc" else b"/sys"
        os.makedirs(path, exist_ok=True)
        if libc.mount(filesystem, path, filesystem, 0, None):
            error = ctypes.get_errno()
            if error != errno.EBUSY:
                raise OSError(error, os.strerror(error), path)
    with open("/dev/vdc", "r+b", buffering=0) as job:
        length = int.from_bytes(job.read(4), "little")
        command = json.loads(job.read(length))
        os.chdir("/app")
        os.environ["VIRTUAL_ENV"] = sys.prefix
        os.environ["PATH"] = os.pathsep.join(
            [os.path.dirname(sys.executable), os.defpath]
        )
        status = 1
        try:
            status = subprocess.run(command, check=False).returncode
        except FileNotFoundError as error:
            print(error, file=sys.stderr, flush=True)
            status = 127
        except PermissionError as error:
            print(error, file=sys.stderr, flush=True)
            status = 126
        if status < 0:
            status = 128 - status
        job.seek(0)
        job.write(b"UVEXIT01" + bytes([status]))
        os.fsync(job.fileno())


try:
    main()
finally:
    if sys.exc_info()[0] is not None:
        traceback.print_exc()
    sys.stdout.flush()
    sys.stderr.flush()
    ctypes.CDLL(None).reboot(POWER_OFF)
